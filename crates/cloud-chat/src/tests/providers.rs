//! A reopened own chat leases a personal agent only after its person explicitly chooses it.
use crate::CloudChat;
use gpui::{Entity, TestAppContext};
use nocterm_agent_chat::conversation::{
    ConversationComposer, ConversationError, ConversationModel,
};
use nocterm_agent_runtime::history_admission::testing::HistoryProbe;
use nocterm_ai::conversation::ConversationAction;
use nocterm_cloud_conversation_model::{
    CommandKey, ConversationAction as CloudAction, ModelName, ProviderOption, RunKey, RunStatus,
    testing::{FakeConversation, run},
};
use std::{rc::Rc, sync::Arc};

fn own(
    cx: &mut TestAppContext,
) -> (
    FakeConversation,
    Entity<CloudChat>,
    Rc<dyn ConversationComposer>,
) {
    let fake = FakeConversation::new("own-provider").with_providers(vec![(
        ProviderOption {
            id: "personal".into(),
            label: "Personal agent".into(),
        },
        ["first", "second"]
            .into_iter()
            .map(|model| ModelName {
                provider: "own-provider".into(),
                model: model.into(),
            })
            .collect(),
    )]);
    let chat = cx
        .update(|cx| CloudChat::open(Arc::new(fake.clone()), HistoryProbe::installation(), cx))
        .unwrap();
    cx.run_until_parked();
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    (fake, chat, composer)
}

#[gpui::test]
async fn the_explicit_own_agent_choice_exposes_every_leased_model_and_prompts_with_the_pick(
    cx: &mut TestAppContext,
) {
    let (fake, chat, composer) = own(cx);
    assert!(cx.update(|cx| composer.can_choose_provider(cx)));
    assert!(!super::view(&chat, cx, |view| view.can_prompt));
    assert_eq!(fake.selected_provider(), None);
    let choices = cx.update(|cx| composer.providers(cx));
    cx.run_until_parked();
    let choices = choices.await.unwrap();
    assert_eq!(choices[0].label, "Personal agent");
    let selection = cx.update(|cx| composer.select_provider(choices[0].id.clone(), cx));
    cx.run_until_parked();
    selection.await.unwrap();
    assert!(super::view(&chat, cx, |view| view.can_prompt));
    assert_eq!(cx.update(|cx| composer.models(cx)).len(), 2);
    assert_eq!(fake.selected_provider().as_deref(), Some("personal"));
    assert!(!cx.update(|cx| composer.can_choose_provider(cx)));
    let selected = ConversationModel {
        provider: "own-provider".into(),
        model: "second".into(),
    };
    cx.update(|cx| composer.select_model(selected, cx)).unwrap();
    let actions = super::actions(&chat, cx);
    let sent = cx.update(|cx| actions.perform(ConversationAction::Prompt("hello".into()), cx));
    cx.run_until_parked();
    assert!(
        matches!(&fake.delivered()[0], CloudAction::Prompt { model: Some(model), .. } if model.provider == "own-provider" && model.model == "second")
    );
    let another = cx.update(|cx| composer.select_provider("another".into(), cx));
    cx.run_until_parked();
    assert!(another.await.is_err());
    drop(sent);
}

#[gpui::test]
async fn an_old_runs_model_does_not_lease_a_provider_and_an_active_run_must_finish_first(
    cx: &mut TestAppContext,
) {
    let (fake, chat, composer) = own(cx);
    fake.set_runs(vec![run(
        RunKey::from_bytes([7; 16]),
        RunStatus::Running,
        CommandKey::from_bytes([7; 16]),
    )]);
    cx.run_until_parked();
    assert_eq!(fake.selected_provider(), None);
    assert!(!super::view(&chat, cx, |view| view.can_prompt));
    assert!(
        cx.update(|cx| composer.can_choose_provider(cx)),
        "the source retains its chooser while a run is active"
    );
    let choices = cx.update(|cx| composer.providers(cx));
    cx.run_until_parked();
    assert_eq!(choices.await.unwrap().len(), 1);
    let busy = cx.update(|cx| composer.select_provider("personal".into(), cx));
    cx.run_until_parked();
    assert!(matches!(busy.await, Err(ConversationError::ProviderBusy)));
    fake.set_runs(vec![run(
        RunKey::from_bytes([7; 16]),
        RunStatus::Completed,
        CommandKey::from_bytes([7; 16]),
    )]);
    cx.run_until_parked();
    let selection = cx.update(|cx| composer.select_provider("personal".into(), cx));
    cx.run_until_parked();
    selection.await.unwrap();
    assert_eq!(
        cx.update(|cx| composer.selected_model(cx))
            .unwrap()
            .provider,
        "own-provider"
    );
}

#[gpui::test]
async fn a_provider_selection_finishing_after_account_end_never_enables_prompting(
    cx: &mut TestAppContext,
) {
    let (fake, _chat, composer) = own(cx);
    let finish = fake.pause_provider_selection();
    let selection = cx.update(|cx| composer.select_provider("personal".into(), cx));
    cx.run_until_parked();
    fake.end();
    cx.run_until_parked();
    let _ = finish.send(());
    cx.run_until_parked();
    assert!(matches!(selection.await, Err(ConversationError::Stale)));
    assert_eq!(fake.selected_provider(), None);
    assert!(cx.update(|cx| composer.revision(cx)).is_err());
    assert!(cx.update(|cx| composer.models(cx)).is_empty());
}

#[gpui::test]
fn a_changed_model_capability_invalidates_the_old_pick_and_the_draft_revision(
    cx: &mut TestAppContext,
) {
    let (fake, chat) = super::opened(cx);
    let models = || {
        ["first", "second"]
            .into_iter()
            .map(|model| ModelName {
                provider: "own".into(),
                model: model.into(),
            })
            .collect()
    };
    fake.set_models(models());
    cx.run_until_parked();
    let composer = cx.update(|cx| CloudChat::conversation(&chat).composer(cx).unwrap());
    cx.update(|cx| {
        composer.select_model(
            ConversationModel {
                provider: "own".into(),
                model: "second".into(),
            },
            cx,
        )
    })
    .unwrap();
    let before = cx.update(|cx| composer.revision(cx)).unwrap();
    fake.set_models(vec![ModelName {
        provider: "own".into(),
        model: "first".into(),
    }]);
    cx.run_until_parked();
    assert!(cx.update(|cx| composer.revision(cx)).unwrap() > before);
    assert_eq!(
        cx.update(|cx| composer.selected_model(cx)).unwrap().model,
        "first"
    );
    assert!(matches!(
        cx.update(|cx| composer.select_model(
            ConversationModel {
                provider: "own".into(),
                model: "second".into()
            },
            cx
        )),
        Err(ConversationError::ModelUnavailable)
    ));
}
