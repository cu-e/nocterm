//! A stopped reading leaves execution uncertainty with the original commands and can reopen.
use super::{actions, opened, view};
use crate::CloudChatError;
use gpui::TestAppContext;
use nocterm_agent_chat::conversation::ConversationError;
use nocterm_ai::conversation::{ConversationAction, PendingPromptState};
use nocterm_cloud_conversation_model::{
    CloudConversationError, CommandKey, CommandState, RowRole, testing::row,
};
use nocterm_core::error::SharedCause;

const FIRST: CommandKey = CommandKey::from_bytes([1; 16]);

fn storage_error() -> CloudConversationError {
    CloudConversationError::Storage(SharedCause::new(std::io::Error::other(
        "private disk failed",
    )))
}

#[gpui::test]
fn an_ended_reading_preserves_its_verified_prefix_and_can_be_opened_again(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    fake.push_rows(vec![row(RowRole::Assistant, None, "verified")]);
    cx.run_until_parked();
    fake.close_reading();
    cx.run_until_parked();
    cx.read(|cx| {
        let chat = chat.read(cx);
        assert!(matches!(
            chat.last_error(),
            Some(CloudChatError::Conversation(CloudConversationError::Ended))
        ));
        assert_eq!(chat.transcript().rows()[0].text, "verified");
    });
    fake.push_rows(vec![row(RowRole::Assistant, None, "later")]);
    cx.update(|cx| chat.update(cx, |chat, cx| chat.set_shown(true, cx).unwrap()));
    cx.run_until_parked();
    assert_eq!(fake.subscribers(), 1);
    cx.read(|cx| {
        let chat = chat.read(cx);
        assert!(chat.last_error().is_none());
        assert_eq!(chat.transcript().rows().len(), 2);
        assert_eq!(chat.thread().entries.len(), 2);
    });
}

#[gpui::test]
async fn a_storage_failure_releases_waiters_without_refusing_an_unanswered_command(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    let sent =
        cx.update(|cx| controls.perform(ConversationAction::Prompt("on its way".into()), cx));
    cx.run_until_parked();
    fake.refuse_round(storage_error());
    fake.close_reading();
    cx.run_until_parked();
    assert!(matches!(sent.await, Err(ConversationError::Unreachable)));
    assert_eq!(
        view(&chat, cx, |view| view.pending_prompts[0].state),
        PendingPromptState::Sending
    );
    cx.read(|cx| {
        assert!(matches!(
            chat.read(cx).last_error(),
            Some(CloudChatError::Conversation(
                CloudConversationError::Storage(_)
            ))
        ))
    });
    cx.update(|cx| chat.update(cx, |chat, cx| chat.set_shown(true, cx).unwrap()));
    cx.run_until_parked();
    fake.settle(FIRST, CommandState::Accepted);
    cx.run_until_parked();
    assert!(view(&chat, cx, |view| view.pending_prompts.is_empty()));
}

#[gpui::test]
async fn a_command_refused_before_storage_stopped_remains_refused_after_eof(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("not sent".into()), cx));
    cx.run_until_parked();
    fake.settle(FIRST, CommandState::Refused);
    fake.refuse_round(storage_error());
    fake.close_reading();
    cx.run_until_parked();
    assert!(matches!(sent.await, Err(ConversationError::Rejected)));
    assert_eq!(
        view(&chat, cx, |view| view.pending_prompts[0].state),
        PendingPromptState::Refused
    );
}

#[gpui::test]
fn a_late_receipt_does_not_install_a_waiter_in_an_ended_reading(cx: &mut TestAppContext) {
    use futures::FutureExt as _;
    let (fake, chat) = opened(cx);
    let release = fake.pause_next_receipt();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("in flight".into()), cx));
    cx.run_until_parked();
    fake.close_reading();
    cx.run_until_parked();
    release.send(()).unwrap();
    cx.run_until_parked();
    assert!(matches!(
        sent.now_or_never(),
        Some(Err(ConversationError::Unreachable))
    ));
    assert_eq!(
        view(&chat, cx, |view| view.pending_prompts[0].state),
        PendingPromptState::Sending
    );
    assert_eq!(fake.subscribers(), 0);
}

#[gpui::test]
fn an_already_delivered_late_receipt_keeps_its_observed_success_after_eof(cx: &mut TestAppContext) {
    use futures::FutureExt as _;
    let (fake, chat) = opened(cx);
    fake.settle_deliveries(Some(CommandState::Delivered));
    let release = fake.pause_next_receipt();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("stored".into()), cx));
    cx.run_until_parked();
    fake.close_reading();
    cx.run_until_parked();
    release.send(()).unwrap();
    cx.run_until_parked();
    assert!(matches!(sent.now_or_never(), Some(Ok(()))));
    assert_eq!(
        view(&chat, cx, |view| view.pending_prompts[0].state),
        PendingPromptState::Delivered
    );
}

#[gpui::test]
fn an_old_account_receipt_cannot_resume_after_the_account_ended(cx: &mut TestAppContext) {
    use futures::FutureExt as _;
    let (fake, chat) = opened(cx);
    let release = fake.pause_next_receipt();
    let controls = actions(&chat, cx);
    let sent =
        cx.update(|cx| controls.perform(ConversationAction::Prompt("old account".into()), cx));
    cx.run_until_parked();
    fake.end();
    cx.run_until_parked();
    release.send(()).unwrap();
    cx.run_until_parked();
    assert!(matches!(
        sent.now_or_never(),
        Some(Err(ConversationError::Stale))
    ));
    cx.read(|cx| assert!(chat.read(cx).transcript().rows().is_empty()));
}
