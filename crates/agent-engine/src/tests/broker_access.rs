use super::{fixtures::*, *};
use crate::{LeaseServices, ModelUsageObserver};
use nocterm_cloud_model::engine::{EngineAction, UsageObservation};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const TOKEN: &str = "nvt1_virtual-token-for-one-chat";

#[derive(Default)]
struct Counters {
    runs: AtomicUsize,
    input: AtomicU64,
    output: AtomicU64,
}
impl ModelUsageObserver for Counters {
    fn run_started(&self) {
        self.runs.fetch_add(1, Ordering::SeqCst);
    }
    fn call_observed(&self, usage: &UsageObservation) {
        self.input
            .fetch_add(usage.input_tokens.unwrap_or(0), Ordering::SeqCst);
        self.output
            .fetch_add(usage.output_tokens.unwrap_or(0), Ordering::SeqCst);
    }
}

fn broker_lease(kind: ProviderKind, root: &str) -> ProviderLease {
    let binding = ProviderBinding::new(
        scope(),
        kind,
        ProviderEndpoint::broker(root).unwrap(),
        vec![NamespaceId::new("fixture-model").unwrap()],
    )
    .unwrap();
    ProviderLease::new(
        binding,
        Zeroizing::new(TOKEN.into()),
        std::time::Duration::from_secs(300),
    )
    .unwrap()
}

#[test]
fn broker_endpoint_allows_private_hosts_but_never_plain_http_to_a_public_name() {
    assert!(ProviderEndpoint::broker("http://127.0.0.1:7000").is_ok());
    assert!(ProviderEndpoint::broker("http://10.0.0.5:7000/v1").is_ok());
    assert!(ProviderEndpoint::broker("https://broker.pool.internal").is_ok());
    assert!(ProviderEndpoint::broker("http://broker.pool.internal").is_err());
    assert!(ProviderEndpoint::broker("http://8.8.8.8").is_err());
    assert!(ProviderEndpoint::broker("https://user:pw@broker.internal").is_err());
    assert!(ProviderEndpoint::broker("ftp://127.0.0.1").is_err());
    assert!(ProviderEndpoint::new("http://127.0.0.1:7000").is_err());
}

#[tokio::test]
async fn upstream_calls_carry_only_the_broker_token_and_usage_is_reported() {
    let server = Server::start(vec![http(&chat_text("answer", true))]).await;
    let lease = broker_lease(ProviderKind::OpenAiChat, server.root());
    let counters = Arc::new(Counters::default());
    lease.attach(&LeaseServices::default().reporting(counters.clone()));
    let mut engine = RigEngineFactory::new(lease)
        .unwrap()
        .create(config(), None)
        .unwrap();
    let result = advance(engine.as_mut()).await.unwrap();
    assert!(matches!(result, EngineAction::Complete(_)));
    let requests = server.requests();
    let headers = requests[0].0.to_ascii_lowercase();
    assert!(headers.contains(&format!("authorization: bearer {TOKEN}")));
    assert!(!headers.contains(KEY));
    assert_eq!(counters.runs.load(Ordering::SeqCst), 1);
    assert_eq!(counters.input.load(Ordering::SeqCst), 11);
    assert_eq!(counters.output.load(Ordering::SeqCst), 7);
}

#[tokio::test]
async fn anthropic_calls_send_the_broker_token_as_the_api_key() {
    let body = anthropic(
        serde_json::json!({"type":"text","text":""}),
        serde_json::json!({"type":"text_delta","text":"answer"}),
        "end_turn",
    );
    let server = Server::start(vec![http(&body)]).await;
    let lease = broker_lease(ProviderKind::AnthropicMessages, server.root());
    let mut engine = RigEngineFactory::new(lease)
        .unwrap()
        .create(config(), None)
        .unwrap();
    let _ = advance(engine.as_mut()).await;
    let requests = server.requests();
    assert!(
        requests[0]
            .0
            .to_ascii_lowercase()
            .contains(&format!("x-api-key: {TOKEN}"))
    );
}
