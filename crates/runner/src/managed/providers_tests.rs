use super::*;
use crate::managed::testing::{Broker, chat, namespace, scope, text_reply, token};
use futures::future::AbortHandle;
use nocterm_cloud_model::{
    authority::RunBinding,
    engine::{EngineAction, EngineInput, EnginePrompt, EngineRequest, ModelBudget},
    identity::{RunEpoch, RunId, RunnerId, RunnerKeyEpoch, StepId},
    testing::{ScriptedEngineEvents, ScriptedModelAdmission},
};
use std::sync::Mutex;

fn boot(root: &str) -> BootV2 {
    boot_with(root, None)
}
fn boot_with(root: &str, policy: Option<serde_json::Value>) -> BootV2 {
    let mut document = serde_json::json!({
            "version": 2, "instance": "i", "agent": "a", "lease": "l", "name": "n",
            "instructions": "", "provider": {"kind": "openai_chat", "base_url": root,
                "models": ["gpt-x"],
                "pricing": {"maximum_request_bytes": 1048576, "maximum_output_tokens": 1024,
                    "maximum_call_cost_micros": 50000}},
            "secret_names": [], "allowed_hosts": [], "ssh_targets": []
    });
    if let Some(policy) = policy {
        document["policy"] = policy;
    }
    BootV2::parse(document.to_string().as_bytes()).unwrap()
}
fn providers(root: &str) -> BrokeredProviders {
    BrokeredProviders::new(&boot(root), &scope(), tokio::runtime::Handle::current()).unwrap()
}
fn providers_with(root: &str, policy: serde_json::Value) -> BrokeredProviders {
    let boot = boot_with(root, Some(policy));
    BrokeredProviders::new(&boot, &scope(), tokio::runtime::Handle::current()).unwrap()
}
fn selection() -> ModelSelection {
    ModelSelection::new(
        namespace(MANAGED_PROVIDER),
        namespace("gpt-x"),
        namespace(MANAGED_PRICING),
    )
}
fn config(chat_byte: u8) -> EngineConfig {
    config_turns(chat_byte, 4)
}
fn config_turns(chat_byte: u8, turns: u32) -> EngineConfig {
    EngineConfig {
        run: RunBinding::new(
            scope(),
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(1).unwrap(),
            chat(chat_byte),
            RunId::from_bytes([5; 16]),
            RunEpoch::new(1).unwrap(),
        ),
        model: namespace("gpt-x"),
        history: Default::default(),
        budget: ModelBudget::new(turns, 1024 * 1024, 256 * 1024, 1024).unwrap(),
    }
}
fn create(
    providers: &BrokeredProviders,
    chat_byte: u8,
) -> Result<Box<dyn AgentEngine>, EngineError> {
    providers
        .model(&selection())
        .unwrap()
        .factory()
        .create(config(chat_byte), None)
}
async fn advance(engine: &mut dyn AgentEngine, chat_byte: u8) -> Result<EngineAction, EngineError> {
    let prompt = EngineRequest::Prompt(EnginePrompt::new(vec!["hello".into()]).unwrap());
    advance_with(engine, chat_byte, StepId::from_bytes([1; 16]), prompt).await
}
async fn advance_with(
    engine: &mut dyn AgentEngine,
    chat_byte: u8,
    next_step: StepId,
    request: EngineRequest,
) -> Result<EngineAction, EngineError> {
    let (_handle, registration) = AbortHandle::new_pair();
    engine
        .advance(
            EngineInput { next_step, request },
            Arc::new(ScriptedEngineEvents::default()),
            Arc::new(ScriptedModelAdmission::new(config(chat_byte).run)),
            registration,
        )
        .await
}

#[path = "execution_tests.rs"]
mod execution;
#[path = "policy_tests.rs"]
mod policy;

#[tokio::test]
async fn a_brokered_model_call_carries_the_chats_own_bearer_token_to_the_broker() {
    let broker = Broker::start(vec![text_reply("done")]).await;
    let providers = providers(broker.root());
    providers.tokens().issue(chat(1), &token("one")).unwrap();
    providers.tokens().issue(chat(2), &token("two")).unwrap();
    let mut engine = create(&providers, 2).unwrap();
    assert!(matches!(
        advance(engine.as_mut(), 2).await,
        Ok(EngineAction::Complete(_))
    ));
    let heads = broker.heads();
    assert_eq!(heads.len(), 1);
    assert!(
        heads[0].contains("authorization: bearer nvt1_two"),
        "{heads:?}"
    );
    assert!(!heads[0].contains("nvt1_one"));
}

struct Woken(Mutex<Vec<CloudChatId>>);
impl ProviderWake for Woken {
    fn wake(&self, chat: CloudChatId) {
        self.0.lock().unwrap().push(chat);
    }
}

#[tokio::test]
async fn a_chat_without_a_token_needs_provider_access_until_issue_wakes_it() {
    let broker = Broker::start(vec![text_reply("done")]).await;
    let providers = providers(broker.root());
    let woken = Arc::new(Woken(Mutex::new(Vec::new())));
    providers.tokens().bind_wake(woken.clone());
    assert!(matches!(
        create(&providers, 1),
        Err(EngineError::NeedsProviderAccess)
    ));
    providers.tokens().issue(chat(1), &token("one")).unwrap();
    assert_eq!(*woken.0.lock().unwrap(), vec![chat(1)]);
    let mut engine = create(&providers, 1).unwrap();
    assert!(advance(engine.as_mut(), 1).await.is_ok());
}

#[tokio::test]
async fn revoking_a_chat_fails_its_next_call_without_reaching_the_broker() {
    let broker = Broker::start(vec![text_reply("done")]).await;
    let providers = providers(broker.root());
    providers.tokens().issue(chat(1), &token("one")).unwrap();
    let mut engine = create(&providers, 1).unwrap();
    providers.tokens().revoke(chat(1));
    assert!(matches!(
        advance(engine.as_mut(), 1).await,
        Err(EngineError::NeedsProviderAccess)
    ));
    assert!(broker.heads().is_empty());
    assert!(matches!(
        create(&providers, 1),
        Err(EngineError::NeedsProviderAccess)
    ));
}

#[tokio::test]
async fn other_providers_pricing_revisions_and_device_grants_are_refused() {
    let providers = providers("http://127.0.0.1:9");
    let other = |provider: &str, model: &str, pricing: &str| {
        ModelSelection::new(namespace(provider), namespace(model), namespace(pricing))
    };
    for refused in [
        other("openai", "gpt-x", MANAGED_PRICING),
        other(MANAGED_PROVIDER, "gpt-x", "v2"),
        other(MANAGED_PROVIDER, "unlisted", MANAGED_PRICING),
    ] {
        assert!(matches!(
            providers.model(&refused),
            Err(EngineError::Unavailable)
        ));
    }
    assert!(providers.model(&selection()).is_ok());
}

#[tokio::test]
async fn a_closed_channel_ends_every_chat_and_refuses_new_tokens() {
    let providers = providers("http://127.0.0.1:9");
    providers.tokens().issue(chat(1), &token("one")).unwrap();
    providers.tokens().close();
    assert!(matches!(
        create(&providers, 1),
        Err(EngineError::NeedsProviderAccess)
    ));
    assert!(providers.tokens().issue(chat(2), &token("two")).is_err());
}

#[tokio::test]
async fn the_debug_form_of_the_token_store_never_shows_a_token() {
    let providers = providers("http://127.0.0.1:9");
    providers
        .tokens()
        .issue(chat(1), &token("secretpart"))
        .unwrap();
    let text = format!("{:?} {:?}", providers.tokens(), token("secretpart"));
    assert!(!text.contains("secretpart"), "{text}");
    assert!(text.contains("chats: 1"));
}
