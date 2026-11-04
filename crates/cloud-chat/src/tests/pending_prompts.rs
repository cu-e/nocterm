//! A sent prompt is shown at once as pending, until the runner accepts it into the
//! transcript, or as failed when nobody answered or the runner refused it.
use super::{actions, opened};
use crate::CloudChat;
use gpui::{Entity, TestAppContext};
use nocterm_agent_chat::conversation::ConversationError;
use nocterm_ai::{
    conversation::{ConversationAction, PendingPromptKey, PendingPromptState},
    thread::Entry,
};
use nocterm_cloud_model::projection::{
    ProjectionError,
    testing::{ProjectionFixture, prompt_event, run_binding},
};

fn pending(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> Vec<(String, PendingPromptState)> {
    let view = CloudChat::conversation(chat);
    let mut prompts = Vec::new();
    cx.update(|cx| {
        view.with_view(cx, &mut |view| {
            prompts = view
                .pending_prompts
                .iter()
                .map(|prompt| (prompt.text.to_owned(), prompt.state))
                .collect();
        })
        .unwrap()
    });
    prompts
}

fn key(chat: &Entity<CloudChat>, cx: &mut TestAppContext) -> PendingPromptKey {
    let view = CloudChat::conversation(chat);
    let mut key = None;
    cx.update(|cx| {
        view.with_view(cx, &mut |view| key = Some(view.pending_prompts[0].key))
            .unwrap()
    });
    key.unwrap()
}

#[gpui::test]
fn a_sent_prompt_is_pending_until_the_runner_accepts_it(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    let delivery = source.pause_delivery();
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("first".into()), cx));
    assert_eq!(
        pending(&chat, cx),
        [("first".to_owned(), PendingPromptState::Sending)],
        "the prompt shows before anything reaches the hub"
    );
    delivery.release();
    cx.run_until_parked();
    drop(sent);
    assert_eq!(
        pending(&chat, cx),
        [("first".to_owned(), PendingPromptState::Delivered)]
    );

    let id = source.effects().staged[0];
    let mut fixture = ProjectionFixture::new();
    source
        .queue(fixture.page(vec![prompt_event(&run_binding(1), "first", id)]))
        .unwrap();
    cx.run_until_parked();
    assert!(
        pending(&chat, cx).is_empty(),
        "the accepted row replaced it"
    );
    cx.update(|cx| {
        assert!(matches!(
            chat.read(cx).projection().transcript().entries.as_slice(),
            [Entry::User(_)]
        ))
    });
}

#[gpui::test]
async fn an_unanswered_prompt_is_failed_and_its_retry_keeps_its_id(cx: &mut TestAppContext) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    source.fail_next_delivery(ProjectionError::Unavailable);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("hello".into()), cx));
    assert!(matches!(sent.await, Err(ConversationError::Unreachable)));
    assert_eq!(
        pending(&chat, cx),
        [("hello".to_owned(), PendingPromptState::Unreachable)]
    );

    let prompt = key(&chat, cx);
    cx.update(|cx| controls.retry_prompt(prompt, cx))
        .await
        .unwrap();
    assert_eq!(source.effects().staged.len(), 1, "the same command again");
    assert_eq!(source.effects().delivered, source.effects().staged);
    assert_eq!(
        pending(&chat, cx),
        [("hello".to_owned(), PendingPromptState::Delivered)]
    );
    assert!(matches!(
        cx.update(|cx| controls.dismiss_prompt(prompt, cx)),
        Err(ConversationError::Unavailable)
    ));
}

#[gpui::test]
async fn a_refused_prompt_stays_refused_until_it_is_sent_again_or_dismissed(
    cx: &mut TestAppContext,
) {
    let (source, chat, _) = opened(cx);
    let controls = actions(&chat, cx);
    source.reject_next_command();
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("no".into()), cx));
    assert!(matches!(sent.await, Err(ConversationError::Rejected)));
    assert_eq!(
        pending(&chat, cx),
        [("no".to_owned(), PendingPromptState::Refused)]
    );
    assert!(
        cx.update(|cx| chat.read(cx).pending_commands().is_empty()),
        "nothing is on its way"
    );

    let refused = key(&chat, cx);
    cx.update(|cx| controls.retry_prompt(refused, cx))
        .await
        .unwrap();
    let staged = source.effects().staged;
    assert_eq!(staged.len(), 2, "the text is a new prompt");
    assert_eq!(
        pending(&chat, cx),
        [("no".to_owned(), PendingPromptState::Delivered)],
        "the new prompt replaced the refused one"
    );

    source.reject_next_command();
    let again = cx.update(|cx| controls.perform(ConversationAction::Prompt("again".into()), cx));
    assert!(matches!(again.await, Err(ConversationError::Rejected)));
    let refused = cx.update(|cx| {
        let mut key = None;
        CloudChat::conversation(&chat)
            .with_view(cx, &mut |view| key = Some(view.pending_prompts[1].key))
            .unwrap();
        key.unwrap()
    });
    cx.update(|cx| controls.dismiss_prompt(refused, cx))
        .unwrap();
    assert_eq!(
        pending(&chat, cx),
        [("no".to_owned(), PendingPromptState::Delivered)]
    );
}
