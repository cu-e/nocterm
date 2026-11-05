//! The remote prompt draft captures images and a model before delivery and fences late clears.
use super::{actions, opened};
use crate::CloudChat;
use gpui::TestAppContext;
use nocterm_agent_chat::conversation::{ConversationError, ConversationModel};
use nocterm_ai::{
    conversation::ConversationAction,
    images::PromptImage,
    testing::{ScriptedImageCodec, images::png_header_fixture},
};
use nocterm_cloud_conversation_model::{CommandKey, CommandState, ModelName};

fn choice(name: &str) -> ConversationModel {
    ConversationModel {
        provider: "managed".into(),
        model: name.into(),
    }
}
fn image(marker: u8) -> PromptImage {
    let mut bytes = png_header_fixture(2, 2, 64);
    bytes[40] = marker;
    let codec = ScriptedImageCodec::default();
    codec.inspection(&bytes, Ok(nocterm_ai::images::preflight(&bytes).unwrap()));
    PromptImage::validate(bytes, &codec).unwrap()
}

#[gpui::test]
fn models_and_images_are_captured_in_the_sent_prompt(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    fake.set_models(vec![
        ModelName {
            provider: "managed".into(),
            model: "a".into(),
        },
        ModelName {
            provider: "managed".into(),
            model: "b".into(),
        },
    ]);
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    assert_eq!(
        cx.update(|cx| composer.models(cx)),
        [choice("a"), choice("b")]
    );
    cx.update(|cx| composer.select_model(choice("b"), cx))
        .unwrap();
    let original = image(7);
    cx.update(|cx| composer.accept_images(vec![original.clone()], cx))
        .unwrap();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("describe".into()), cx));
    cx.run_until_parked();
    let delivered = fake.delivered();
    let nocterm_cloud_conversation_model::ConversationAction::Prompt { model, images, .. } =
        &delivered[0]
    else {
        panic!("prompt")
    };
    assert_eq!(model.as_ref().unwrap().model, "b");
    assert_eq!(images[0].bytes.as_ref(), original.bytes());
    assert!(
        cx.update(|cx| composer.select_model(choice("missing"), cx))
            .is_err()
    );
    drop(sent);
}

#[gpui::test]
fn a_late_send_acknowledgement_preserves_newer_images(cx: &mut TestAppContext) {
    let (_, chat) = opened(cx);
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    cx.update(|cx| composer.accept_images(vec![image(1)], cx))
        .unwrap();
    let revision = cx.update(|cx| composer.revision(cx)).unwrap();
    cx.update(|cx| composer.accept_images(vec![image(2)], cx))
        .unwrap();
    assert!(matches!(
        cx.update(|cx| composer.clear_images(revision, cx)),
        Err(ConversationError::Stale)
    ));
    assert_eq!(cx.update(|cx| composer.images(cx)).len(), 2);
    let current = cx.update(|cx| composer.revision(cx)).unwrap();
    cx.update(|cx| composer.clear_images(current, cx)).unwrap();
    assert!(cx.update(|cx| composer.images(cx)).is_empty());
}

#[gpui::test]
async fn retry_preserves_the_original_images_and_model_after_the_draft_changes(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    fake.set_models(vec![
        ModelName {
            provider: "managed".into(),
            model: "a".into(),
        },
        ModelName {
            provider: "managed".into(),
            model: "b".into(),
        },
    ]);
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    cx.update(|cx| composer.select_model(choice("a"), cx))
        .unwrap();
    cx.update(|cx| composer.accept_images(vec![image(3)], cx))
        .unwrap();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("again".into()), cx));
    cx.run_until_parked();
    fake.settle(CommandKey::from_bytes([1; 16]), CommandState::Refused);
    cx.run_until_parked();
    assert!(matches!(sent.await, Err(ConversationError::Rejected)));
    cx.update(|cx| composer.select_model(choice("b"), cx))
        .unwrap();
    cx.update(|cx| composer.remove_image(0, cx)).unwrap();
    let key = super::view(&chat, cx, |view| view.pending_prompts[0].key);
    let resent = cx.update(|cx| controls.retry_prompt(key, cx));
    cx.run_until_parked();
    assert_eq!(fake.delivered()[0], fake.delivered()[1]);
    drop(resent);
}

#[gpui::test]
fn installed_observers_share_the_source_owned_composer_and_keep_account_fencing(
    cx: &mut TestAppContext,
) {
    let fake = nocterm_cloud_conversation_model::testing::FakeConversation::new("installed");
    fake.set_models(vec![ModelName {
        provider: "managed".into(),
        model: "a".into(),
    }]);
    let installation = cx.update(|cx| {
        crate::CloudChatInstallation::new(
            nocterm_agent_runtime::history_admission::testing::HistoryProbe::installation(),
            cx,
        )
    });
    let first = cx
        .update(|cx| installation.open(std::sync::Arc::new(fake.clone()), cx))
        .unwrap();
    let second = cx
        .update(|cx| installation.open(std::sync::Arc::new(fake.clone()), cx))
        .unwrap();
    let first_draft = cx.update(|cx| first.composer(cx).unwrap());
    let second_draft = cx.update(|cx| second.composer(cx).unwrap());
    cx.update(|cx| first_draft.accept_images(vec![image(4)], cx))
        .unwrap();
    assert_eq!(cx.update(|cx| second_draft.images(cx)).len(), 1);
    fake.end();
    cx.run_until_parked();
    assert!(cx.update(|cx| first_draft.revision(cx)).is_err());
    assert!(
        cx.update(|cx| second_draft.accept_images(vec![image(5)], cx))
            .is_err()
    );
    assert!(cx.update(|cx| second_draft.images(cx)).is_empty());
}

#[gpui::test]
async fn a_restored_failed_prompt_without_its_body_never_sends_a_text_only_replacement(
    cx: &mut TestAppContext,
) {
    use nocterm_cloud_conversation_model::CloudConversation as _;
    let (fake, chat) = opened(cx);
    let command = fake
        .deliver(
            nocterm_cloud_conversation_model::ConversationAction::Prompt {
                text: "original".into(),
                server: None,
                model: Some(ModelName {
                    provider: "managed".into(),
                    model: "a".into(),
                }),
                images: vec![nocterm_cloud_conversation_model::PromptImage {
                    mime: nocterm_cloud_conversation_model::ImageMime::Png,
                    bytes: [1, 2, 3].as_slice().into(),
                }],
            },
        )
        .await
        .unwrap();
    fake.settle(command, CommandState::Refused);
    cx.run_until_parked();
    let controls = actions(&chat, cx);
    let key = super::view(&chat, cx, |view| view.pending_prompts[0].key);
    assert!(matches!(
        cx.update(|cx| controls.retry_prompt(key, cx)).await,
        Err(ConversationError::OriginalPromptUnavailable)
    ));
    assert_eq!(fake.delivered().len(), 1);
    assert_eq!(super::view(&chat, cx, |view| view.pending_prompts.len()), 1);
}

#[gpui::test]
async fn a_failed_retry_delivery_keeps_the_original_body_and_row(cx: &mut TestAppContext) {
    let (fake, chat) = opened(cx);
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    cx.update(|cx| composer.accept_images(vec![image(7)], cx))
        .unwrap();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("retry".into()), cx));
    cx.run_until_parked();
    fake.settle(CommandKey::from_bytes([1; 16]), CommandState::Refused);
    cx.run_until_parked();
    assert!(sent.await.is_err());
    let key = super::view(&chat, cx, |view| view.pending_prompts[0].key);
    fake.refuse_deliveries(|| {
        nocterm_cloud_conversation_model::CloudConversationError::Unavailable
    });
    let retry = cx.update(|cx| controls.retry_prompt(key, cx));
    cx.run_until_parked();
    assert!(retry.await.is_err());
    assert_eq!(
        super::view(&chat, cx, |view| view.pending_prompts[0].key),
        key
    );
    let retry = cx.update(|cx| controls.retry_prompt(key, cx));
    cx.run_until_parked();
    assert!(retry.await.is_err());
    let sent = fake.delivered();
    assert_eq!(sent[0], sent[1]);
    assert_eq!(sent[0], sent[2]);
}

#[gpui::test]
async fn an_immediate_delivery_failure_keeps_the_body_before_projection_arrives(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    fake.settle_deliveries(Some(CommandState::Unreachable));
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    cx.update(|cx| composer.accept_images(vec![image(8)], cx))
        .unwrap();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("immediate".into()), cx));
    cx.run_until_parked();
    assert!(matches!(sent.await, Err(ConversationError::Unreachable)));
    fake.settle_deliveries(None);
    let key = super::view(&chat, cx, |view| view.pending_prompts[0].key);
    let retry = cx.update(|cx| controls.retry_prompt(key, cx));
    cx.run_until_parked();
    assert_eq!(fake.delivered().len(), 2);
    assert_eq!(fake.delivered()[0], fake.delivered()[1]);
    drop(retry);
}

#[gpui::test]
async fn retry_captures_the_implicit_model_before_a_later_run_changes_the_default(
    cx: &mut TestAppContext,
) {
    use nocterm_cloud_conversation_model::{RunKey, RunStatus, testing::run};
    let (fake, chat) = opened(cx);
    fake.set_models(vec![
        ModelName {
            provider: "managed".into(),
            model: "model-a".into(),
        },
        ModelName {
            provider: "other-provider".into(),
            model: "model-b".into(),
        },
    ]);
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt("implicit".into()), cx));
    cx.run_until_parked();
    fake.settle(CommandKey::from_bytes([1; 16]), CommandState::Refused);
    cx.run_until_parked();
    assert!(sent.await.is_err());
    let key = super::view(&chat, cx, |view| view.pending_prompts[0].key);
    let mut later = run(
        RunKey::from_bytes([9; 16]),
        RunStatus::Completed,
        CommandKey::from_bytes([9; 16]),
    );
    later.model = ModelName {
        provider: "other-provider".into(),
        model: "model-b".into(),
    };
    fake.set_runs(vec![later]);
    cx.run_until_parked();
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    assert_eq!(
        cx.update(|cx| composer.selected_model(cx)).unwrap().model,
        "model-b"
    );
    let retry = cx.update(|cx| controls.retry_prompt(key, cx));
    cx.run_until_parked();
    let sent = fake.delivered();
    assert_eq!(sent[0], sent[1]);
    assert!(
        matches!(&sent[1], nocterm_cloud_conversation_model::ConversationAction::Prompt { model: Some(model), .. } if model.model == "model-a" && model.provider == "managed")
    );
    drop(retry);
}

#[gpui::test]
fn an_image_only_pending_prompt_has_a_description_without_changing_its_sent_text(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    cx.update(|cx| composer.accept_images(vec![image(1)], cx))
        .unwrap();
    let controls = actions(&chat, cx);
    let sent = cx.update(|cx| controls.perform(ConversationAction::Prompt(String::new()), cx));
    assert_eq!(
        super::view(&chat, cx, |view| view.pending_prompts[0].text.to_owned()),
        "Image attachment"
    );
    cx.run_until_parked();
    assert_eq!(
        super::view(&chat, cx, |view| view.pending_prompts[0].text.to_owned()),
        "Image attachment"
    );
    assert!(
        matches!(&fake.delivered()[0], nocterm_cloud_conversation_model::ConversationAction::Prompt { text, images, .. } if text.is_empty() && images.len() == 1)
    );
    drop(sent);
}

#[gpui::test]
async fn an_old_hubs_large_prompt_refusal_preserves_the_captured_draft_and_update_guidance(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = opened(cx);
    fake.refuse_deliveries(|| {
        nocterm_cloud_conversation_model::CloudConversationError::LargePromptUnsupported(
            nocterm_core::SharedCause::new(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "fragmented publication unsupported",
            )),
        )
    });
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    cx.update(|cx| composer.accept_images(vec![image(2)], cx))
        .unwrap();
    let revision = cx.update(|cx| composer.revision(cx)).unwrap();
    let controls = actions(&chat, cx);
    let (submission, sent) = cx
        .update(|cx| controls.submit(ConversationAction::Prompt("describe".into()), cx))
        .unwrap();
    cx.run_until_parked();
    let error = sent.await.unwrap_err();
    assert!(matches!(
        error,
        ConversationError::LargePromptUnsupported(_)
    ));
    assert!(error.to_string().contains("Update the Cloud hub"));
    assert!(cx.update(|cx| submission.refused(cx)));
    assert_eq!(submission.pending_prompt(), None);
    let retry = cx.update(|cx| submission.retry(cx));
    assert!(matches!(retry.await, Err(ConversationError::Unavailable)));
    assert_eq!(cx.update(|cx| composer.revision(cx)).unwrap(), revision);
    assert_eq!(cx.update(|cx| composer.images(cx)).len(), 1);
    assert!(super::view(&chat, cx, |view| view
        .pending_prompts
        .is_empty()));
}
