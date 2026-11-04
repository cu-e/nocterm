//! A runner that does not answer (a sleeping seat) and one that refuses: the chat stays usable
//! and says which it is.
use super::{actions, opened, view_phase};
use crate::{CloudChat, PendingCommandState};
use gpui::{Entity, TestAppContext};
use nocterm_agent_chat::conversation::ConversationError;
use nocterm_ai::conversation::{ConversationAction, ConversationPhase};
use nocterm_cloud_model::{
    coordinator::{CoordinatorEvent, RunStatus},
    identity::CommandId,
    projection::{
        ProjectionError,
        testing::{ProjectionFixture, prompt_event, run_binding},
    },
};

fn reconnecting(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> bool {
    let view = CloudChat::conversation(chat);
    let mut reconnecting = false;
    cx.update(|cx| {
        view.with_view(cx, &mut |view| reconnecting = view.reconnecting)
            .unwrap()
    });
    reconnecting
}

fn rows(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> usize {
    cx.update(|cx| chat.read(cx).projection().transcript().entries.len())
}

#[gpui::test]
fn an_unanswered_replay_poll_reconnects_and_shows_the_next_page(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    let mut fixture = ProjectionFixture::new();
    source.fail_next_replay(ProjectionError::Unavailable);
    source
        .queue(fixture.page(vec![prompt_event(
            &run_binding(1),
            "first",
            CommandId::from_bytes([1; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(rows(&chat, cx), 1);
    assert!(reconnecting(&chat, cx), "the next poll went unanswered");
    assert!(cx.update(|cx| chat.read(cx).last_error().is_some()));

    source
        .queue(fixture.page(vec![CoordinatorEvent::Status {
            run: run_binding(1),
            status: RunStatus::Stopped,
        }]))
        .unwrap();
    cx.executor().advance_clock(crate::replay::FIRST_RETRY);
    cx.run_until_parked();
    assert_eq!(
        view_phase(CloudChat::conversation(&chat).as_ref(), cx),
        ConversationPhase::Stopped,
        "the replay went on after its delay"
    );
    assert!(!reconnecting(&chat, cx));
    assert!(cx.update(|cx| chat.read(cx).last_error().is_none()));
}

#[gpui::test]
async fn an_unreachable_runner_keeps_the_prompt_for_a_retry_and_says_so(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    source.fail_next_delivery(ProjectionError::Unavailable);
    let (submission, sent) = cx
        .update(|cx| controls.submit(ConversationAction::Prompt("привет".into()), cx))
        .unwrap();
    assert!(matches!(sent.await, Err(ConversationError::Unreachable)));
    let pending = cx.update(|cx| chat.read(cx).pending_commands());
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].state(), PendingCommandState::OutcomeUnknown);
    assert!(
        !cx.update(|cx| submission.refused(cx)),
        "a retry can still deliver it"
    );

    cx.update(|cx| submission.retry(cx)).await.unwrap();
    let id = pending[0].id();
    assert_eq!(
        source.effects().staged,
        [id],
        "the retry keeps the original command"
    );
    assert_eq!(source.effects().delivered, [id]);
}

#[gpui::test]
async fn a_refused_prompt_is_named_so_the_next_send_is_new(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    source.reject_next_command();
    let (submission, sent) = cx
        .update(|cx| controls.submit(ConversationAction::Prompt("refused".into()), cx))
        .unwrap();
    assert!(matches!(sent.await, Err(ConversationError::Rejected)));
    assert!(cx.update(|cx| chat.read(cx).pending_commands().is_empty()));
    assert!(cx.update(|cx| submission.refused(cx)));
    assert!(matches!(
        cx.update(|cx| submission.retry(cx)).await,
        Err(ConversationError::Rejected)
    ));
}
