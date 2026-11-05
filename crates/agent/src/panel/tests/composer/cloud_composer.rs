//! Cloud composer controls send the chosen model and images through the original source.
use super::cloud_delivery::{open_source, press, send, shown};
use super::*;
use nocterm_cloud_conversation_model::testing::FakeConversation;
use nocterm_cloud_conversation_model::{
    CommandKey, CommandState, ConversationAction as CloudAction, ModelName,
};

fn model(name: &str) -> ModelName {
    ModelName {
        provider: "openai-compatible".into(),
        model: name.into(),
    }
}
fn pick_image(f: &Fixture, cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image.png");
    std::fs::write(&path, image_bytes()).unwrap();
    press(f, "agent-attach-image", cx);
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.multiple && !options.directories);
        Some(vec![path])
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn cloud_prompts_send_images_without_text_using_the_model_chosen_in_the_composer(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let source =
        FakeConversation::new("image-model").with_models(vec![model("first"), model("second")]);
    let (facade, _) = open_source(&f, source.clone(), cx);
    press(&f, "cloud-model-picker", cx);
    press(&f, "cloud-model-openai-compatible-second", cx);
    pick_image(&f, cx);
    assert!(shown(&f, "agent-image-strip", cx));
    send(&f, "", cx);
    let actions = source.delivered();
    let CloudAction::Prompt {
        text,
        model: selected,
        images,
        ..
    } = &actions[0]
    else {
        panic!("expected prompt")
    };
    assert!(text.is_empty());
    assert_eq!(selected, &Some(model("second")));
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].bytes.as_ref(), image_bytes());
    source.settle(CommandKey::from_bytes([1; 16]), CommandState::Delivered);
    cx.run_until_parked();
    assert!(!shown(&f, "agent-image-strip", cx));
    cx.update(|cx| assert!(facade.composer(cx).unwrap().images(cx).is_empty()));
}

#[gpui_kit::test]
fn images_added_while_an_older_prompt_is_pending_are_kept_for_the_next_send(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, _) = cloud_delivery::captured_panel(&f, cx);
    send(&f, "question", cx);
    pick_image(&f, cx);
    source.settle(CommandKey::from_bytes([1; 16]), CommandState::Delivered);
    cx.run_until_parked();
    assert!(shown(&f, "agent-image-strip", cx));
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "question"));
    send(&f, "next image", cx);
    let actions = source.delivered();
    let CloudAction::Prompt { images, text, .. } = &actions[1] else {
        panic!("expected next prompt")
    };
    assert_eq!(text, "next image");
    assert_eq!(images.len(), 1);
}

#[gpui_kit::test]
fn repeating_a_prompt_refused_before_tracking_keeps_its_text_and_images(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let source = FakeConversation::new("pre-admission-refusal");
    source.refuse_deliveries(|| nocterm_cloud_conversation_model::CloudConversationError::Capacity);
    let (facade, _) = open_source(&f, source.clone(), cx);
    pick_image(&f, cx);
    send(&f, "unchanged question", cx);
    assert!(shown(&f, "agent-error", cx));
    assert!(shown(&f, "agent-image-strip", cx));
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "unchanged question"
        )
    });
    // Do not rewrite the input: this must create another real admission attempt.
    press(&f, "agent-send", cx);
    let attempted = source.delivered();
    assert_eq!(attempted.len(), 2);
    assert_eq!(attempted[0], attempted[1]);
    assert!(
        matches!(&attempted[1], CloudAction::Prompt { text, images, .. } if text == "unchanged question" && images.len() == 1)
    );
    assert!(shown(&f, "agent-error", cx));
    assert!(shown(&f, "agent-image-strip", cx));
    cx.update(|cx| {
        assert_eq!(
            f.panel.read(cx).composer_text(cx).as_ref(),
            "unchanged question"
        );
        assert_eq!(facade.composer(cx).unwrap().images(cx).len(), 1);
        facade
            .with_view(cx, &mut |view| assert!(view.pending_prompts.is_empty()))
            .unwrap();
    });
}

#[gpui_kit::test]
fn changing_the_server_sends_the_same_text_again_and_the_old_ack_keeps_the_new_draft(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let (source, _) = cloud_delivery::captured_panel(&f, cx);
    source.set_servers(super::cloud_servers::binding());
    cx.run_until_parked();
    press(
        &f,
        gpui_kit::ElementId::Name(format!("cloud-target-{}", super::cloud_servers::DB).into()),
        cx,
    );
    pick_image(&f, cx);
    send(&f, "same question", cx);
    press(&f, "cloud-target-default", cx);
    // Keep the exact input ticket: changing only the server creates a new prompt.
    press(&f, "agent-send", cx);
    let delivered = source.delivered();
    assert_eq!(delivered.len(), 2);
    assert!(matches!(
        &delivered[0],
        CloudAction::Prompt { text, images, server: Some(server), .. }
            if text == "same question" && images.len() == 1 && *server == super::cloud_servers::DB
    ));
    assert!(matches!(
        &delivered[1],
        CloudAction::Prompt { text, images, server: None, .. }
            if text == "same question" && images.len() == 1
    ));
    source.settle(CommandKey::from_bytes([1; 16]), CommandState::Delivered);
    cx.run_until_parked();
    assert!(shown(&f, "agent-image-strip", cx));
    cx.update(|cx| assert_eq!(f.panel.read(cx).composer_text(cx).as_ref(), "same question"));
}

#[gpui_kit::test]
fn a_late_image_picker_cannot_attach_to_a_reopened_conversation(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let source = FakeConversation::new("picker-original");
    let (facade, selector) = open_source(&f, source, cx);
    press(&f, "agent-attach-image", cx);
    cx.update(|cx| selector.close(cx).unwrap());
    cx.update_window(f.handle, |_, window, cx| {
        selector
            .open(facade.clone(), "Reopened".into(), window, cx)
            .unwrap()
    })
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("old.png");
    std::fs::write(&path, image_bytes()).unwrap();
    cx.simulate_path_prompt_response(|_| Some(vec![path]));
    cx.run_until_parked();
    assert!(!shown(&f, "agent-image-strip", cx));
    cx.update(|cx| assert!(facade.composer(cx).unwrap().images(cx).is_empty()));
}

#[gpui_kit::test]
fn removing_a_cloud_image_uses_the_same_composer_control(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let (_, _) = cloud_delivery::captured_panel(&f, cx);
    pick_image(&f, cx);
    assert!(shown(&f, "agent-image-strip", cx));
    press(&f, ("remove-image", 0usize), cx);
    assert!(!shown(&f, "agent-image-strip", cx));
}

#[gpui_kit::test]
fn reopening_an_own_chat_explicitly_chooses_a_personal_agent_before_its_models(
    cx: &mut TestAppContext,
) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    let source = FakeConversation::new("own-reopened").with_providers(vec![(
        nocterm_cloud_conversation_model::ProviderOption {
            id: "personal".into(),
            label: "Personal agent".into(),
        },
        vec![model("first"), model("second")],
    )]);
    let (facade, _) = open_source(&f, source.clone(), cx);
    assert!(shown(&f, "cloud-provider-picker", cx));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        let picker = window.find("cloud-provider-picker").bounds();
        assert!(picker.right() <= window.find("agent-send").bounds().origin.x);
        assert!(picker.origin.x >= window.find("agent-composer").bounds().origin.x);
    })
    .unwrap();
    press(&f, "agent-send", cx);
    assert!(source.delivered().is_empty());
    press(&f, "cloud-provider-picker", cx);
    press(&f, "cloud-provider-option-personal", cx);
    assert!(shown(&f, "cloud-model-picker", cx));
    assert!(!shown(&f, "cloud-provider-picker", cx));
    cx.update(|cx| {
        facade
            .with_view(cx, &mut |view| assert!(view.can_prompt))
            .unwrap()
    });
    press(&f, "cloud-model-picker", cx);
    press(&f, "cloud-model-openai-compatible-second", cx);
    send(&f, "from history", cx);
    let actions = source.delivered();
    let CloudAction::Prompt {
        model: selected, ..
    } = &actions[0]
    else {
        panic!("expected prompt")
    };
    assert_eq!(selected, &Some(model("second")));
}

#[gpui_kit::test]
fn a_late_personal_agent_choice_cannot_authorize_an_ended_account(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let source = FakeConversation::new("ended-own").with_providers(vec![(
        nocterm_cloud_conversation_model::ProviderOption {
            id: "personal".into(),
            label: "Personal".into(),
        },
        vec![model("first")],
    )]);
    let ready = source.pause_provider_selection();
    let (facade, _) = open_source(&f, source.clone(), cx);
    press(&f, "cloud-provider-picker", cx);
    press(&f, "cloud-provider-option-personal", cx);
    source.end();
    cx.run_until_parked();
    let _ = ready.send(());
    cx.run_until_parked();
    assert!(source.selected_provider().is_none());
    assert!(source.delivered().is_empty());
    cx.update(|cx| assert!(facade.composer(cx).unwrap().revision(cx).is_err()));
}

#[gpui_kit::test]
fn a_personal_agent_can_be_chosen_after_the_reopened_run_finishes(cx: &mut TestAppContext) {
    use nocterm_cloud_conversation_model::testing::run;
    use nocterm_cloud_conversation_model::{RunKey, RunStatus};
    let f = fixture(cx);
    let source = FakeConversation::new("running-own").with_providers(vec![(
        nocterm_cloud_conversation_model::ProviderOption {
            id: "personal".into(),
            label: "Personal".into(),
        },
        vec![model("first")],
    )]);
    let key = RunKey::from_bytes([7; 16]);
    let command = CommandKey::from_bytes([6; 16]);
    source.set_runs(vec![run(key, RunStatus::Running, command)]);
    let (_, _) = open_source(&f, source.clone(), cx);
    press(&f, "cloud-provider-picker", cx);
    press(&f, "cloud-provider-option-personal", cx);
    assert!(shown(&f, "cloud-provider-error", cx));
    assert!(source.selected_provider().is_none());
    source.set_runs(vec![run(key, RunStatus::Completed, command)]);
    cx.run_until_parked();
    press(&f, "cloud-provider-option-personal", cx);
    assert_eq!(source.selected_provider().as_deref(), Some("personal"));
    assert!(shown(&f, "cloud-model-picker", cx));
}

#[gpui_kit::test]
fn choosing_a_model_during_image_preparation_keeps_the_image_for_that_model(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let source = FakeConversation::new("image-preparation-model")
        .with_models(vec![model("first"), model("second")]);
    let (_, _) = open_source(&f, source.clone(), cx);
    let (ready, wait) = futures::channel::oneshot::channel();
    f.panel.update(cx, |panel, cx| {
        panel.prepare_images(
            async move {
                wait.await.unwrap();
                Ok(vec![prompt_image()])
            },
            cx,
        )
    });
    press(&f, "cloud-model-picker", cx);
    press(&f, "cloud-model-openai-compatible-second", cx);
    ready.send(()).unwrap();
    cx.run_until_parked();
    assert!(shown(&f, "agent-image-strip", cx));
    send(&f, "describe", cx);
    let actions = source.delivered();
    let CloudAction::Prompt {
        model: selected,
        images,
        ..
    } = &actions[0]
    else {
        panic!("expected prompt")
    };
    assert_eq!(selected, &Some(model("second")));
    assert_eq!(images.len(), 1);
}

#[gpui_kit::test]
fn an_old_image_remove_action_never_removes_the_image_that_shifted_into_its_place(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    let source = FakeConversation::new("shared-images");
    let (facade, _) = open_source(&f, source, cx);
    cx.update(|cx| {
        let composer = facade.composer(cx).unwrap();
        composer
            .accept_images(vec![prompt_image(), prompt_image()], cx)
            .unwrap();
        let obsolete = crate::panel::conversation_surface::RenderedImageRemoval::capture(
            facade.as_ref(),
            0,
            cx,
        )
        .unwrap();
        composer.remove_image(0, cx).unwrap();
        obsolete.activate(cx).unwrap();
        assert_eq!(composer.images(cx).len(), 1);
        let current = crate::panel::conversation_surface::RenderedImageRemoval::capture(
            facade.as_ref(),
            0,
            cx,
        )
        .unwrap();
        current.activate(cx).unwrap();
        assert!(composer.images(cx).is_empty());
    });
}
