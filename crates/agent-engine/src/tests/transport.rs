use super::{fixtures::*, *};
use nocterm_cloud_model::engine::{EngineAction, EngineEvent};
use serde_json::json;
use std::net::IpAddr;

#[test]
fn provider_endpoint_rejects_cleartext_embedded_credentials_and_private_addresses() {
    for root in [
        "http://api.openai.com/v1",
        "https://key@api.example/v1",
        "https://api.example/v1?key=secret",
        "https://api.example/#fragment",
        "https://127.0.0.1/v1",
        "https://10.0.0.1",
        "https://[::1]",
        "https://[::ffff:127.0.0.1]",
    ] {
        assert!(ProviderEndpoint::new(root).is_err(), "{root}");
    }
    assert!(ProviderEndpoint::new("https://api.openai.com/v1").is_ok());
    assert!(ProviderEndpoint::new("https://api.anthropic.com").is_ok());
}
#[test]
fn dns_answers_cannot_rebind_to_internal_reserved_or_transition_addresses() {
    for ip in [
        "0.0.0.0",
        "10.1.2.3",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "192.168.1.1",
        "192.0.2.1",
        "198.18.1.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "::1",
        "fc00::1",
        "fe80::1",
        "::ffff:10.0.0.1",
        "2002:0a00:0001::1",
        "2001:db8::1",
        "2001::1",
    ] {
        assert!(
            !crate::http::is_public(ip.parse::<IpAddr>().unwrap()),
            "{ip}"
        );
    }
    assert!(crate::http::is_public("8.8.8.8".parse().unwrap()));
    assert!(crate::http::is_public("2606:4700::1111".parse().unwrap()));
}
#[tokio::test]
async fn redirect_does_not_forward_api_key_or_follow_the_new_destination() {
    let target = Server::start(vec![http(&chat_text("unexpected", true))]).await;
    let response = format!(
        "HTTP/1.1 307 Temporary Redirect\r\nLocation: {}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        target.root()
    );
    let origin = Server::start(vec![response]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, origin.root());
    assert!(advance(engine.as_mut()).await.is_err());
    assert_eq!(origin.requests().len(), 1);
    assert!(target.requests().is_empty());
}
#[tokio::test]
async fn response_advertising_an_excessive_size_is_refused_before_body_read() {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 33554433\r\nConnection: close\r\n\r\n".into();
    let server = Server::start(vec![response]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    assert!(advance(engine.as_mut()).await.is_err());
}
#[tokio::test]
async fn text_progress_is_bounded_without_splitting_unicode() {
    let text = "я".repeat(20_000);
    let server = Server::start(vec![http(&chat_text(&text, true))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let events = Arc::new(Events::default());
    let (_, registration) = AbortHandle::new_pair();
    assert!(matches!(
        engine
            .advance(input(1), events.clone(), admission(), registration)
            .await
            .unwrap(),
        EngineAction::Complete(_)
    ));
    let captured = events.events();
    let fragments: Vec<_> = captured
        .iter()
        .filter_map(|event| match event {
            EngineEvent::Text(text) => Some(text.text()),
            _ => None,
        })
        .collect();
    assert!(fragments.iter().all(|s| s.len() <= 16 * 1024));
    assert_eq!(fragments.concat(), text);
}
#[tokio::test]
async fn different_account_or_model_cannot_use_the_installed_provider_access() {
    let server = Server::start(vec![]).await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let mut changed = config();
    changed.model = NamespaceId::new("unconfigured-model").unwrap();
    assert!(factory.create(changed, None).is_err());
    let mut changed = config();
    let other = AccountScope::new(
        scope().hub().clone(),
        AccountIdentity::new(
            NamespaceId::new("other-account").unwrap(),
            Fingerprint::from_bytes([2; 32]),
        ),
    );
    changed.run = RunBinding::new(
        other,
        changed.run.runner(),
        changed.run.runner_epoch(),
        changed.run.chat(),
        changed.run.run(),
        changed.run.epoch(),
    );
    assert!(factory.create(changed, None).is_err());
    assert!(server.requests().is_empty());
    assert!(!format!("{factory:?}").contains(KEY));
}
#[tokio::test]
async fn model_byte_budget_refuses_excessive_output_and_never_dispatches_tools() {
    let server = Server::start(vec![http(&chat_tools(
        &json!({"terminal_id":"session","program":"printf"}).to_string(),
        1,
        true,
    ))])
    .await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let mut config = config();
    config.budget = ModelBudget::new(4, 1024 * 1024, 1, 1024).unwrap();
    let mut engine = factory.create(config, None).unwrap();
    assert!(matches!(
        advance(engine.as_mut()).await,
        Err(EngineError::BudgetExceeded)
    ));
}
