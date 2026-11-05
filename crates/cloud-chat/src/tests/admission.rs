//! Admission refusals preserve the verified transcript and the rows the renderer reads.
use super::{opened, view};
use crate::{CloudChat, CloudChatError};
use gpui::{Entity, TestAppContext};
use nocterm_agent_runtime::history_admission::HistoryAdmissionError;
use nocterm_cloud_conversation_model::{ConversationDelta, RowRole, testing::row};

const REJECTED_TEXT_BYTES: usize = 20 * 1024 * 1024;

fn accepted(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> (Vec<String>, Vec<String>) {
    let neutral = cx.read(|cx| {
        chat.read(cx)
            .transcript()
            .rows()
            .iter()
            .map(|row| row.text.clone())
            .collect()
    });
    let rendered = view(chat, cx, |view| {
        view.transcript
            .entries
            .iter()
            .filter_map(|entry| match entry {
                nocterm_ai::thread::Entry::Agent(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    });
    (neutral, rendered)
}
fn assert_refused(chat: &Entity<CloudChat>, cx: &mut TestAppContext) {
    cx.read(|cx| {
        assert!(matches!(
            chat.read(cx).last_error(),
            Some(CloudChatError::History(
                HistoryAdmissionError::ResidentCapacity { .. }
            ))
        ));
    });
}

#[gpui::test]
fn refused_transcript_growth_preserves_the_accepted_rows(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    fake.push_rows(vec![row(RowRole::Assistant, None, "accepted")]);
    cx.run_until_parked();
    let before = accepted(&chat, cx);
    fake.push_rows(vec![row(
        RowRole::Assistant,
        None,
        &"x".repeat(REJECTED_TEXT_BYTES),
    )]);
    cx.run_until_parked();
    assert_refused(&chat, cx);
    assert_eq!(accepted(&chat, cx), before);
    assert_eq!(fake.subscribers(), 0, "refused growth ends its reading");
}

#[gpui::test]
fn a_refused_replacement_keeps_the_old_text_and_can_resume_from_a_small_reset(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    fake.push_rows(vec![row(RowRole::Assistant, None, "accepted")]);
    cx.run_until_parked();
    fake.replace_last(row(
        RowRole::Assistant,
        None,
        &"x".repeat(REJECTED_TEXT_BYTES),
    ));
    cx.run_until_parked();
    assert_refused(&chat, cx);
    assert_eq!(
        accepted(&chat, cx),
        (vec!["accepted".into()], vec!["accepted".into()])
    );
    fake.apply_delta(ConversationDelta {
        reset: true,
        changed_from: Some(0),
        rows: vec![row(RowRole::Assistant, None, "restored")],
        ..ConversationDelta::default()
    });
    cx.update(|cx| chat.update(cx, |chat, cx| chat.set_shown(true, cx).unwrap()));
    cx.run_until_parked();
    cx.read(|cx| assert!(chat.read(cx).last_error().is_none()));
    assert_eq!(
        accepted(&chat, cx),
        (vec!["restored".into()], vec!["restored".into()])
    );
}

#[gpui::test]
fn retained_windows_and_resets_charge_only_the_rows_they_keep(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    // Ten MiB per row fits, but counting a removed row would reject the next ten MiB.
    let text = "x".repeat(10 * 1024 * 1024);
    fake.push_rows(vec![row(RowRole::Assistant, None, &text)]);
    cx.run_until_parked();
    fake.apply_delta(ConversationDelta {
        first_index: 1,
        ..ConversationDelta::default()
    });
    cx.run_until_parked();
    assert!(accepted(&chat, cx).0.is_empty());
    fake.push_rows(vec![row(RowRole::Assistant, None, &text)]);
    cx.run_until_parked();
    cx.read(|cx| assert!(chat.read(cx).last_error().is_none()));
    fake.apply_delta(ConversationDelta {
        reset: true,
        changed_from: Some(0),
        rows: vec![row(RowRole::Assistant, None, "reset")],
        ..ConversationDelta::default()
    });
    cx.run_until_parked();
    assert_eq!(
        accepted(&chat, cx),
        (vec!["reset".into()], vec!["reset".into()])
    );
}

#[gpui::test]
fn a_reopened_reading_accepts_the_latest_window_instead_of_old_queued_updates(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    fake.push_rows(vec![row(RowRole::Assistant, None, "accepted")]);
    cx.run_until_parked();
    for index in 0..64 {
        fake.replace_last(row(RowRole::Assistant, None, &format!("queued-{index}")));
    }
    cx.update(|cx| chat.update(cx, |chat, cx| chat.set_shown(false, cx).unwrap()));
    fake.apply_delta(ConversationDelta {
        reset: true,
        changed_from: Some(0),
        rows: vec![row(RowRole::Assistant, None, "reopened")],
        ..ConversationDelta::default()
    });
    cx.update(|cx| chat.update(cx, |chat, cx| chat.set_shown(true, cx).unwrap()));
    cx.run_until_parked();
    assert_eq!(
        accepted(&chat, cx),
        (vec!["reopened".into()], vec!["reopened".into()])
    );
}

#[gpui::test]
fn suffix_replacement_keeps_the_surviving_order_even_for_unordered_indices(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    let mut first = row(RowRole::Assistant, None, "two");
    first.index = 2;
    let mut last = row(RowRole::Assistant, None, "one");
    last.index = 1;
    fake.apply_delta(ConversationDelta {
        changed_from: Some(0),
        rows: vec![first.clone(), last],
        ..ConversationDelta::default()
    });
    cx.run_until_parked();
    first.text = "replacement".into();
    fake.apply_delta(ConversationDelta {
        changed_from: Some(2),
        rows: vec![first],
        ..ConversationDelta::default()
    });
    cx.run_until_parked();
    let expected = vec!["one".into(), "replacement".into()];
    assert_eq!(accepted(&chat, cx), (expected.clone(), expected));
}
