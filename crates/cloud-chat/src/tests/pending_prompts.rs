//! A sent prompt shows at once as pending, until the runner accepts it into the transcript,
//! or as failed when nobody answered or the hub or runner refused it (ADR 0134).
use super::{actions, opened, view};
use crate::CloudChat;
use gpui::{Entity, TestAppContext};
use nocterm_agent_chat::conversation::ConversationError;
use nocterm_ai::conversation::{ConversationAction, PendingPromptKey, PendingPromptState};
use nocterm_ai::thread::Entry;
use nocterm_cloud_conversation_model::{CommandKey, CommandState, RunKey, RunStatus};

fn pending(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> Vec<(String, PendingPromptState)> {
    view(chat, cx, |view| {
        view.pending_prompts
            .iter()
            .map(|prompt| (prompt.text.to_owned(), prompt.state))
            .collect()
    })
}

fn first_key(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> PendingPromptKey {
    view(chat, cx, |view| view.pending_prompts[0].key)
}

/// The first command the fake tracks.
const FIRST: CommandKey = CommandKey::from_bytes([1; 16]);

#[gpui::test]
async fn a_sent_prompt_shows_at_once_and_follows_its_command_until_accepted(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("first".into()), cx));
    assert_eq!(
        pending(&chat, cx),
        [("first".to_owned(), PendingPromptState::Sending)],
        "the prompt shows before the conversation has it"
    );
    cx.run_until_parked();
    assert_eq!(
        pending(&chat, cx),
        [("first".to_owned(), PendingPromptState::Sending)],
        "one row, now the tracked command's"
    );
    fake.settle(FIRST, CommandState::Delivered);
    cx.run_until_parked();
    sent.await.unwrap();
    assert_eq!(
        pending(&chat, cx),
        [("first".to_owned(), PendingPromptState::Delivered)]
    );
    fake.accept_prompt(FIRST, RunKey::from_bytes([3; 16]), RunStatus::Queued);
    cx.run_until_parked();
    assert!(
        pending(&chat, cx).is_empty(),
        "the accepted row replaced it"
    );
    cx.update(|cx| {
        assert!(matches!(
            chat.read(cx).thread().entries.as_slice(),
            [Entry::User(_)]
        ))
    });
}

#[gpui::test]
async fn a_refused_prompt_stays_until_it_is_sent_again_as_a_new_prompt(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("again".into()), cx));
    cx.run_until_parked();
    fake.settle(FIRST, CommandState::Refused);
    cx.run_until_parked();
    assert!(matches!(sent.await, Err(ConversationError::Rejected)));
    assert_eq!(
        pending(&chat, cx),
        [("again".to_owned(), PendingPromptState::Refused)]
    );
    let key = first_key(&chat, cx);
    let resent = cx.update(|cx| controls.retry_prompt(key, cx));
    cx.run_until_parked();
    assert_eq!(
        pending(&chat, cx),
        [("again".to_owned(), PendingPromptState::Sending)],
        "the new prompt replaced the refused row"
    );
    assert_eq!(fake.delivered().len(), 2);
    drop(resent);
}

#[gpui::test]
async fn an_unanswered_prompt_is_shown_as_unreachable_and_can_be_dismissed(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("lost".into()), cx));
    cx.run_until_parked();
    fake.settle(FIRST, CommandState::Unreachable);
    cx.run_until_parked();
    assert!(matches!(sent.await, Err(ConversationError::Unreachable)));
    assert_eq!(
        pending(&chat, cx),
        [("lost".to_owned(), PendingPromptState::Unreachable)]
    );
    let key = first_key(&chat, cx);
    cx.update(|cx| controls.dismiss_prompt(key, cx)).unwrap();
    assert!(pending(&chat, cx).is_empty());
    assert!(matches!(
        cx.update(|cx| controls.dismiss_prompt(key, cx)),
        Err(ConversationError::Unavailable)
    ));
}

#[gpui::test]
fn at_most_four_refused_prompts_are_shown(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    for index in 1..=6u8 {
        let sent =
            cx.update(|cx| controls.perform(ConversationAction::Prompt(format!("p{index}")), cx));
        cx.run_until_parked();
        fake.settle(CommandKey::from_bytes([index; 16]), CommandState::Refused);
        cx.run_until_parked();
        drop(sent);
    }
    let shown: Vec<_> = pending(&chat, cx)
        .into_iter()
        .map(|(text, _)| text)
        .collect();
    assert_eq!(shown, ["p3", "p4", "p5", "p6"]);
}

#[gpui::test]
fn a_conversation_that_refuses_the_prompt_leaves_no_row(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    fake.refuse_deliveries(|| nocterm_cloud_conversation_model::CloudConversationError::NoModel);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("x".into()), cx));
    cx.run_until_parked();
    drop(sent);
    assert!(pending(&chat, cx).is_empty());
}

#[gpui::test]
async fn a_hidden_chat_pauses_after_its_last_pending_command_settles(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("finish".into()), cx));
    cx.run_until_parked();
    cx.update(|cx| chat.update(cx, |chat, cx| chat.set_shown(false, cx).unwrap()));
    cx.run_until_parked();
    assert_eq!(
        fake.subscribers(),
        1,
        "the pending command still needs its answer"
    );
    fake.settle(FIRST, CommandState::Accepted);
    cx.run_until_parked();
    sent.await.unwrap();
    assert_eq!(
        fake.subscribers(),
        0,
        "a settled hidden chat no longer reads"
    );
}
