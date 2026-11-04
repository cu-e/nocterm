//! Stop, Allow and approval answers name what the chat shows, and go to its conversation.
use super::{actions, opened, view};
use gpui::TestAppContext;
use nocterm_agent_chat::conversation::{ConversationError, ToolApprovalChoice};
use nocterm_ai::conversation::{ConversationAction, ConversationPhase};
use nocterm_cloud_conversation_model::testing::{run, tool_row};
use nocterm_cloud_conversation_model::{
    Approval, CommandKey, ConversationAction as CloudAction, IntentKey, RunKey, RunStatus,
    ToolState,
};

const RUN: RunKey = RunKey::from_bytes([3; 16]);

#[gpui::test]
async fn an_own_prompt_waiting_for_its_grant_offers_allow_for_its_run(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("go".into()), cx));
    cx.run_until_parked();
    drop(sent);
    fake.accept_prompt(CommandKey::from_bytes([1; 16]), RUN, RunStatus::NeedsGrant);
    cx.run_until_parked();
    assert_eq!(
        view(&chat, cx, |view| view.phase),
        ConversationPhase::NeedsGrant
    );
    assert!(view(&chat, cx, |view| view.can_allow));
    // Actions captured before the run existed never act on it.
    let stale = cx.update(|cx| controls.perform(ConversationAction::Allow, cx));
    assert!(matches!(stale.await, Err(ConversationError::Stale)));
    let controls = actions(&chat, cx);
    let allowed = cx.update(|cx| controls.perform(ConversationAction::Allow, cx));
    cx.run_until_parked();
    drop(allowed);
    assert_eq!(
        fake.delivered().last(),
        Some(&CloudAction::Allow { run: RUN })
    );
    assert!(
        !view(&chat, cx, |view| view.can_allow),
        "a grant on its way is not offered twice"
    );
}

#[gpui::test]
fn a_run_of_another_device_offers_no_allow_and_stop_names_the_shown_run(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    fake.set_runs(vec![run(
        RUN,
        RunStatus::NeedsGrant,
        CommandKey::from_bytes([9; 16]),
    )]);
    cx.run_until_parked();
    assert!(!view(&chat, cx, |view| view.can_allow));
    assert!(view(&chat, cx, |view| view.can_stop));
    let controls = actions(&chat, cx);
    let stopped = cx.update(|cx| controls.perform(ConversationAction::Stop, cx));
    cx.run_until_parked();
    drop(stopped);
    assert_eq!(fake.delivered(), [CloudAction::Stop { run: Some(RUN) }]);
}

#[gpui::test]
async fn an_approval_is_answered_for_its_intent_and_refuses_a_session_wide_allow(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    let intent = IntentKey::from_bytes([5; 16]);
    fake.push_rows(vec![tool_row(
        RUN,
        intent,
        "exec_command",
        ToolState::AwaitingApproval { expires_at_ms: 99 },
    )]);
    fake.set_approvals(vec![Approval {
        intent,
        run: RUN,
        expires_at_ms: 99,
    }]);
    cx.run_until_parked();
    let source = nocterm_cloud_chat_source(&chat);
    let approvals = cx.update(|cx| source.tool_approvals(cx).unwrap());
    assert_eq!(approvals.len(), 1);
    let session = cx.update(|cx| approvals[0].choose(ToolApprovalChoice::AllowForSession, cx));
    let answered = cx.update(|cx| approvals[0].choose(ToolApprovalChoice::AllowOnce, cx));
    cx.run_until_parked();
    drop(answered);
    assert!(matches!(session.await, Err(ConversationError::Unavailable)));
    assert_eq!(
        fake.delivered(),
        [CloudAction::Answer {
            intent,
            approve: true
        }]
    );
    // A newer control revision makes the card stale.
    fake.set_control_revision(7);
    cx.run_until_parked();
    assert!(matches!(
        cx.update(|cx| approvals[0].with_view(cx, &mut |_| {})),
        Err(ConversationError::Unavailable)
    ));
}

fn nocterm_cloud_chat_source(
    chat: &gpui::Entity<crate::CloudChat>,
) -> std::rc::Rc<dyn nocterm_agent_chat::conversation::ConversationSource> {
    crate::CloudChat::conversation(chat)
}
