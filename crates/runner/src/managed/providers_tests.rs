use super::*;
use crate::managed::testing::{chat, namespace, scope, token};
use futures::future::AbortHandle;
use nocterm_cloud_model::{
    authority::RunBinding,
    engine::{EngineAction, EngineInput, EnginePrompt, EngineRequest, ModelBudget},
    identity::{RunEpoch, RunId, RunnerId, RunnerKeyEpoch, StepId},
    testing::{ScriptedEngineEvents, ScriptedModelAdmission},
};
use std::sync::Mutex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// A broker stub that records the head of each request and answers one SSE chat.
struct Broker {
    root: String,
    heads: Arc<Mutex<Vec<String>>>,
}
impl Broker {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let heads = Arc::new(Mutex::new(Vec::new()));
        let seen = heads.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 4096];
                let split = loop {
                    let read = stream.read(&mut chunk).await.unwrap();
                    bytes.extend_from_slice(&chunk[..read]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let head = String::from_utf8_lossy(&bytes[..split]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .map_or(0, |n| n.trim().parse::<usize>().unwrap());
                while bytes.len() < split + length {
                    let read = stream.read(&mut chunk).await.unwrap();
                    bytes.extend_from_slice(&chunk[..read]);
                }
                seen.lock().unwrap().push(head);
                stream.write_all(reply().as_bytes()).await.unwrap();
                let _ = stream.shutdown().await;
            }
        });
        Self { root, heads }
    }
    fn heads(&self) -> Vec<String> {
        self.heads.lock().unwrap().clone()
    }
}
fn reply() -> String {
    let data = |value: serde_json::Value| format!("data: {value}\n\n");
    let body = data(
        serde_json::json!({"id":"m","object":"chat.completion.chunk","model":"gpt-x",
        "created":0,"choices":[{"index":0,"delta":{"role":"assistant","content":"done"},
        "finish_reason":null}]}),
    ) + &data(
        serde_json::json!({"id":"m","object":"chat.completion.chunk","model":"gpt-x",
            "created":0,"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
    ) + &data(
        serde_json::json!({"id":"m","object":"chat.completion.chunk","model":"gpt-x",
            "choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}),
    ) + "data: [DONE]\n\n";
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn boot(root: &str) -> BootV2 {
    BootV2::parse(
        serde_json::json!({
            "version": 2, "instance": "i", "agent": "a", "lease": "l", "name": "n",
            "instructions": "", "provider": {"kind": "openai_chat", "base_url": root,
                "models": ["gpt-x"],
                "pricing": {"maximum_request_bytes": 1048576, "maximum_output_tokens": 1024,
                    "maximum_call_cost_micros": 50000}},
            "secret_names": [], "allowed_hosts": [], "ssh_targets": []
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}
fn providers(root: &str) -> BrokeredProviders {
    BrokeredProviders::new(&boot(root), &scope(), tokio::runtime::Handle::current()).unwrap()
}
fn selection() -> ModelSelection {
    ModelSelection::new(
        namespace(MANAGED_PROVIDER),
        namespace("gpt-x"),
        namespace(MANAGED_PRICING),
    )
}
fn config(chat_byte: u8) -> EngineConfig {
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
        budget: ModelBudget::new(4, 1024 * 1024, 256 * 1024, 1024).unwrap(),
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
    let (_handle, registration) = AbortHandle::new_pair();
    engine
        .advance(
            EngineInput {
                next_step: StepId::from_bytes([1; 16]),
                request: EngineRequest::Prompt(EnginePrompt::new(vec!["hello".into()]).unwrap()),
            },
            Arc::new(ScriptedEngineEvents::default()),
            Arc::new(ScriptedModelAdmission::new(config(chat_byte).run)),
            registration,
        )
        .await
}

#[tokio::test]
async fn a_brokered_model_call_carries_the_chats_own_bearer_token_to_the_broker() {
    let broker = Broker::start().await;
    let providers = providers(&broker.root);
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
    let broker = Broker::start().await;
    let providers = providers(&broker.root);
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
    let broker = Broker::start().await;
    let providers = providers(&broker.root);
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
