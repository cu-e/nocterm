use crate::CloudChat;
use gpui::{Entity, TestAppContext};
use nocterm_agent_chat::conversation::{ConversationActions, ConversationError};
use nocterm_agent_runtime::history_admission::testing::HistoryProbe;
use nocterm_ai::conversation::{ConversationAction, ConversationFreshness, ConversationPhase};
use nocterm_ai::thread::Entry;
use nocterm_cloud_conversation_model::testing::{FakeConversation, row, session_row, tool_row};
use nocterm_cloud_conversation_model::{
    CloudConversationError, Freshness, IntentKey, RowRole, RunKey, SessionState, ToolOutcome,
    ToolState,
};
use std::{rc::Rc, sync::Arc};

mod admission;
mod commands;
mod composer;
mod pending_prompts;
mod providers;
mod reading;
mod servers;

pub(super) fn opened(cx: &mut TestAppContext) -> (FakeConversation, Entity<CloudChat>) {
    let fake = FakeConversation::new("runner-chat-1");
    let chat = cx.update(|cx| {
        CloudChat::open(Arc::new(fake.clone()), HistoryProbe::installation(), cx).unwrap()
    });
    cx.run_until_parked();
    (fake, chat)
}

pub(super) fn actions(
    chat: &Entity<CloudChat>,
    cx: &mut TestAppContext,
) -> Rc<dyn ConversationActions> {
    cx.update(|cx| CloudChat::conversation(chat).capture_actions(cx).unwrap())
}

/// Reads one value of the chat's view.
pub(super) fn view<T>(
    chat: &Entity<CloudChat>,
    cx: &mut TestAppContext,
    read: impl Fn(&nocterm_ai::conversation::ConversationView<'_>) -> T,
) -> T {
    let source = CloudChat::conversation(chat);
    let mut value = None;
    cx.update(|cx| {
        source
            .with_view(cx, &mut |view| value = Some(read(&view)))
            .unwrap()
    });
    value.unwrap()
}

#[gpui::test]
fn a_chat_shows_its_verified_rows_including_the_sessions_its_runner_opened(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    let run = Some(RunKey::from_bytes([1; 16]));
    fake.push_rows(vec![
        row(RowRole::User, run, "deploy"),
        row(RowRole::Assistant, run, "on it"),
        session_row(
            RunKey::from_bytes([1; 16]),
            [9; 16],
            "deploy",
            SessionState::Established,
        ),
    ]);
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = chat.read(cx).thread();
        assert!(matches!(
            thread.entries.as_slice(),
            [Entry::User(_), Entry::Agent(text), Entry::Tool(session)]
                if text == "on it" && session.title.contains("SSH") && session.title.contains("connected")
        ));
    });
    // A streamed row grows in place; the earlier entries stay.
    fake.replace_last(row(RowRole::Assistant, run, "done"));
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = chat.read(cx).thread();
        assert_eq!(thread.entries.len(), 3);
        assert!(matches!(&thread.entries[2], Entry::Agent(text) if text == "done"));
    });
}

#[gpui::test]
fn a_tool_row_shows_the_title_and_the_typed_outcome_the_core_gave_it(cx: &mut TestAppContext) {
    use nocterm_ai::tool_display::{ToolDisplay, ToolOutcome as Shown};
    let (fake, chat) = opened(cx);
    let run = RunKey::from_bytes([1; 16]);
    let mut failed = tool_row(
        run,
        IntentKey::from_bytes([2; 16]),
        "run_command",
        ToolState::Failed,
    );
    if let Some(tool) = failed.tool.as_mut() {
        tool.title = "web-1 · Run command".to_owned();
        tool.outcome = Some(ToolOutcome::Err("exit 2".to_owned()));
    }
    let mut done = tool_row(
        run,
        IntentKey::from_bytes([3; 16]),
        "read_command",
        ToolState::Succeeded,
    );
    if let Some(tool) = done.tool.as_mut() {
        tool.outcome = Some(ToolOutcome::Ok("{\"lines\":3}".to_owned()));
    }
    fake.push_rows(vec![failed, done]);
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = chat.read(cx).thread();
        let [Entry::Tool(failed), Entry::Tool(done)] = thread.entries.as_slice() else {
            panic!("two tool rows");
        };
        assert_eq!(failed.title, "web-1 · Run command");
        assert!(matches!(
            ToolDisplay::from_call(failed).and_then(|display| display.outcome),
            Some(Shown::Err(text)) if text == "exit 2"
        ));
        assert_eq!(done.title, "server · read_command");
        assert!(matches!(
            ToolDisplay::from_call(done).and_then(|display| display.outcome),
            Some(Shown::Ok(value)) if value["lines"] == 3
        ));
    });
}

#[gpui::test]
async fn a_sleeping_seat_shows_its_stored_history_as_unconfirmed_and_offers_wake(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    // The subscription's first freshness is `Opening`: nothing is claimed yet, so a stored
    // history is not shown as unconfirmed.
    assert_eq!(
        view(&chat, cx, |view| view.freshness),
        ConversationFreshness::Live
    );
    fake.set_freshness(Freshness::Opening {
        last_witnessed_at_ms: Some(3),
    });
    cx.run_until_parked();
    assert_eq!(
        view(&chat, cx, |view| view.freshness),
        ConversationFreshness::Live
    );
    fake.set_freshness(Freshness::Unconfirmed {
        last_witnessed_at_ms: None,
    });
    cx.run_until_parked();
    assert_eq!(
        view(&chat, cx, |view| view.freshness),
        ConversationFreshness::Unconfirmed { can_wake: true }
    );
    let controls = actions(&chat, cx);
    let woken = cx.update(|cx| controls.wake(cx));
    woken.await.unwrap();
    cx.run_until_parked();
    assert_eq!(fake.wakes(), 1);
    fake.set_freshness(Freshness::Current { witnessed_at_ms: 5 });
    cx.run_until_parked();
    assert_eq!(
        view(&chat, cx, |view| view.freshness),
        ConversationFreshness::Live
    );
    fake.set_freshness(Freshness::Withheld { witnessed_at_ms: 6 });
    cx.run_until_parked();
    assert_eq!(
        view(&chat, cx, |view| view.freshness),
        ConversationFreshness::Withheld
    );
}

#[gpui::test]
fn an_unanswered_round_reconnects_until_the_next_change(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    fake.refuse_round(CloudConversationError::Unavailable);
    cx.run_until_parked();
    assert!(view(&chat, cx, |view| view.reconnecting));
    assert!(cx.update(|cx| chat.read(cx).last_error().is_some()));
    fake.push_rows(vec![row(RowRole::Assistant, None, "back")]);
    cx.run_until_parked();
    assert!(!view(&chat, cx, |view| view.reconnecting));
}

#[gpui::test]
fn window_observers_share_one_reading_and_the_last_hidden_one_pauses_it(cx: &mut TestAppContext) {
    let fake = FakeConversation::new("shared");
    let installed =
        cx.update(|cx| crate::CloudChatInstallation::new(HistoryProbe::installation(), cx));
    let first = cx.update(|cx| installed.open(Arc::new(fake.clone()), cx).unwrap());
    let second = cx.update(|cx| installed.open(Arc::new(fake.clone()), cx).unwrap());
    cx.run_until_parked();
    assert_eq!(fake.subscribers(), 1, "one chat reads for both windows");
    cx.update(|cx| first.set_shown(false, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(fake.subscribers(), 1);
    cx.update(|cx| second.set_shown(false, cx).unwrap());
    cx.run_until_parked();
    assert_eq!(fake.subscribers(), 0, "nobody looks: reading pauses");
    fake.push_rows(vec![row(RowRole::Assistant, None, "while hidden")]);
    cx.update(|cx| first.set_shown(true, cx).unwrap());
    cx.run_until_parked();
    let mut rows = 0;
    cx.update(|cx| {
        first
            .with_view(cx, &mut |view| rows = view.transcript.entries.len())
            .unwrap()
    });
    assert_eq!(
        rows, 1,
        "the reopened reading starts with the whole transcript"
    );
}

#[gpui::test]
async fn a_chat_whose_account_ended_is_stale_and_refuses_actions(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    fake.end();
    cx.run_until_parked();
    let source = CloudChat::conversation(&chat);
    cx.update(|cx| {
        assert!(matches!(
            source.with_view(cx, &mut |_| {}),
            Err(ConversationError::Stale)
        ))
    });
    let refused = cx.update(|cx| controls.perform(ConversationAction::Prompt("late".into()), cx));
    assert!(matches!(refused.await, Err(ConversationError::Stale)));
    assert!(fake.delivered().is_empty());
}

#[gpui::test]
fn a_deleted_chat_shows_deleted_and_takes_no_prompt(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    fake.set_deleted();
    cx.run_until_parked();
    assert_eq!(
        view(&chat, cx, |view| view.phase),
        ConversationPhase::Deleted
    );
    assert!(!view(&chat, cx, |view| view.can_prompt));
}
