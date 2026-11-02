//! The person's permission for a queued run to use the model (ADR 0129).
use super::{actions, opened, view_phase};
use crate::CloudChat;
use gpui::{Entity, TestAppContext};
use nocterm_agent_chat::conversation::ConversationError;
use nocterm_ai::conversation::{ConversationAction, ConversationPhase};
use nocterm_cloud_model::{
    coordinator::{CoordinatorEvent, RunStatus},
    identity::CommandId,
    projection::{
        ConversationIntent,
        ports::CommandDelivery,
        testing::{ProjectionFixture, ScriptedConversationSource, prompt_event, run_binding},
    },
};
use std::sync::Arc;

/// Every staged command that grants the run model use.
fn staged_grants(source: &ScriptedConversationSource) -> Vec<CommandId> {
    source
        .effects()
        .staged
        .into_iter()
        .filter(|id| {
            source
                .staged(*id)
                .is_some_and(|command| matches!(command.action, ConversationIntent::GrantInference))
        })
        .collect()
}
fn can_allow(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> bool {
    let view = CloudChat::conversation(chat);
    let mut allowed = false;
    cx.update(|cx| {
        view.with_view(cx, &mut |view| allowed = view.can_allow)
            .unwrap()
    });
    allowed
}
/// Sends a prompt and replays the runner's acceptance of it as a queued run.
async fn accepted_prompt(
    source: &Arc<ScriptedConversationSource>,
    chat: &Entity<CloudChat>,
    fixture: &mut ProjectionFixture,
    cx: &mut TestAppContext,
) {
    let controls = actions(chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("hello".into()), cx));
    sent.await.unwrap();
    let id = cx.update(|cx| chat.read(cx).pending_commands()[0].id());
    source
        .runner_receipt(id, CommandDelivery::Accepted)
        .unwrap();
    source
        .queue(fixture.page(vec![prompt_event(&run_binding(1), "hello", id)]))
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
async fn sending_a_prompt_allows_its_run_once_and_the_run_leaves_the_waiting_state(
    cx: &mut TestAppContext,
) {
    let (source, chat, _) = opened(cx);
    let mut fixture = ProjectionFixture::new();
    accepted_prompt(&source, &chat, &mut fixture, cx).await;
    let grants = staged_grants(&source);
    assert_eq!(grants.len(), 1);
    let grant = source.staged(grants[0]).unwrap();
    assert_eq!(grant.coordinates.run, Some(run_binding(1)));
    assert!(source.effects().delivered.contains(&grants[0]));
    // The grant is on its way; the person is not offered a second one.
    assert!(!can_allow(&chat, cx));
    let view = CloudChat::conversation(&chat);
    assert_eq!(view_phase(view.as_ref(), cx), ConversationPhase::Queued);
    source
        .queue(fixture.page(vec![CoordinatorEvent::Status {
            run: run_binding(1),
            status: RunStatus::Running,
        }]))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(view_phase(view.as_ref(), cx), ConversationPhase::Running);
    assert_eq!(staged_grants(&source), grants);
}

#[gpui::test]
async fn a_run_nobody_allowed_waits_for_one_explicit_allow(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    let mut fixture = ProjectionFixture::new();
    // The prompt came from elsewhere: no pending prompt of ours was accepted.
    source
        .queue(fixture.page(vec![prompt_event(
            &run_binding(1),
            "elsewhere",
            CommandId::from_bytes([77; 16]),
        )]))
        .unwrap();
    cx.run_until_parked();
    assert!(staged_grants(&source).is_empty());
    assert!(can_allow(&chat, cx));
    let controls = actions(&chat, cx);
    cx.update(|cx| controls.perform(ConversationAction::Allow, cx))
        .await
        .unwrap();
    let grants = staged_grants(&source);
    assert_eq!(grants.len(), 1);
    assert_eq!(
        source.staged(grants[0]).unwrap().coordinates.run,
        Some(run_binding(1))
    );
    let again = cx.update(|cx| controls.perform(ConversationAction::Allow, cx));
    assert!(matches!(again.await, Err(ConversationError::Unavailable)));
    assert_eq!(staged_grants(&source).len(), 1);
}

#[gpui::test]
async fn allow_is_refused_while_no_run_waits(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    assert!(!can_allow(&chat, cx));
    let controls = actions(&chat, cx);
    let refused = cx.update(|cx| controls.perform(ConversationAction::Allow, cx));
    assert!(matches!(refused.await, Err(ConversationError::Unavailable)));
    assert!(source.effects().staged.is_empty());
}
