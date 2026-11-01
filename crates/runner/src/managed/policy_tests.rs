//! The boot policy as the managed model path enforces it: the tools the model
//! is offered, the run limits and the spend ceilings (ADR 0127).
use super::*;
use crate::managed::testing::{exec_reply, text_reply};
use nocterm_cloud_model::{
    engine::{EngineToolResult, EngineToolResults, ModelCallRequest},
    inference::InferenceError,
    payload::BoundedJson,
};

const EXEC: &str = r#"{"terminal_id":"t","program":"/bin/true","args":[]}"#;

fn only_reading() -> serde_json::Value {
    serde_json::json!({"tools": {"allowed": ["read_terminal", "no_such_tool"]}})
}
async fn issued(broker: &Broker, policy: serde_json::Value) -> BrokeredProviders {
    let providers = providers_with(broker.root(), policy);
    providers.tokens().issue(chat(1), &token("one")).unwrap();
    providers
}

#[tokio::test]
async fn a_tool_outside_the_policy_is_not_offered_to_the_model() {
    let broker = Broker::start(vec![text_reply("a"), text_reply("b")]).await;
    let narrow = issued(&broker, only_reading()).await;
    let mut engine = create(&narrow, 1).unwrap();
    advance(engine.as_mut(), 1).await.unwrap();
    let offered = &broker.bodies()[0];
    assert!(offered.contains("read_terminal"), "{offered}");
    assert!(!offered.contains("exec_command"), "{offered}");

    let open = issued(&broker, serde_json::json!({"tools": {"allowed": "all"}})).await;
    let mut engine = create(&open, 1).unwrap();
    advance(engine.as_mut(), 1).await.unwrap();
    assert!(broker.bodies()[1].contains("exec_command"));
}

const TRANSFER_TOOLS: [&str; 3] = ["transfer_files", "read_transfer", "cancel_transfer"];

/// A tool is offered when it is a declared function, not merely mentioned in another description.
fn offers(body: &str, tool: &str) -> bool {
    body.contains(&format!(r#""function":{{"name":"{tool}""#))
}

#[tokio::test]
async fn the_transfer_tools_are_offered_to_all_and_to_a_list_that_names_them() {
    let broker = Broker::start(vec![text_reply("a"), text_reply("b")]).await;
    let open = issued(&broker, serde_json::json!({"tools": {"allowed": "all"}})).await;
    let mut engine = create(&open, 1).unwrap();
    advance(engine.as_mut(), 1).await.unwrap();
    for name in TRANSFER_TOOLS {
        assert!(offers(&broker.bodies()[0], name), "{name} missing for all");
    }

    let named = serde_json::json!({"tools": {"allowed": ["transfer_files", "read_transfer"]}});
    let narrow = issued(&broker, named).await;
    let mut engine = create(&narrow, 1).unwrap();
    advance(engine.as_mut(), 1).await.unwrap();
    let offered = &broker.bodies()[1];
    assert!(offers(offered, "transfer_files") && offers(offered, "read_transfer"));
    assert!(!offers(offered, "cancel_transfer"), "{offered}");
    assert!(!offers(offered, "exec_command"), "{offered}");
}

#[tokio::test]
async fn an_empty_allow_list_offers_no_tool_and_the_model_can_still_answer() {
    let broker = Broker::start(vec![text_reply("just words")]).await;
    let providers = issued(&broker, serde_json::json!({"tools": {"allowed": []}})).await;
    let mut engine = create(&providers, 1).unwrap();
    assert!(matches!(
        advance(engine.as_mut(), 1).await,
        Ok(EngineAction::Complete(_))
    ));
    assert!(!broker.bodies()[0].contains("read_terminal"));
}

#[tokio::test]
async fn a_call_to_a_tool_outside_the_policy_is_refused() {
    let broker = Broker::start(vec![exec_reply(EXEC)]).await;
    let providers = issued(&broker, only_reading()).await;
    let mut engine = create(&providers, 1).unwrap();
    assert!(advance(engine.as_mut(), 1).await.is_err());
    let allowed = issued(
        &broker,
        serde_json::json!({"tools": {"allowed": ["exec_command"]}}),
    )
    .await;
    let mut engine = create(&allowed, 1).unwrap();
    assert!(matches!(
        advance(engine.as_mut(), 1).await,
        Ok(EngineAction::Tools(_))
    ));
}

async fn tool_round(engine: &mut dyn AgentEngine) -> Result<EngineAction, EngineError> {
    let EngineAction::Tools(batch) = advance(engine, 1).await? else {
        panic!("the broker asked for a tool");
    };
    let output = BoundedJson::new(serde_json::json!({"output": "ok"})).unwrap();
    let results = EngineToolResults::new(
        batch.step(),
        vec![EngineToolResult::new(
            batch.calls()[0].slot(),
            output,
            false,
        )],
    )
    .unwrap();
    advance_with(
        engine,
        1,
        StepId::from_bytes([2; 16]),
        EngineRequest::ToolResults(results),
    )
    .await
}

#[tokio::test]
async fn max_turns_stops_the_run_before_another_model_call() {
    let broker = Broker::start(vec![exec_reply(EXEC), text_reply("never asked")]).await;
    let policy = serde_json::json!({"tools": {"allowed": "all"}, "limits": {"max_turns": 1}});
    let providers = issued(&broker, policy).await;
    let mut engine = create(&providers, 1).unwrap();
    assert!(matches!(
        tool_round(engine.as_mut()).await,
        Ok(EngineAction::Complete(_))
    ));
    assert_eq!(broker.bodies().len(), 1);
}

#[tokio::test]
async fn a_policy_never_widens_the_limits_of_the_command() {
    let broker = Broker::start(vec![exec_reply(EXEC), text_reply("never asked")]).await;
    let policy = serde_json::json!({"tools": {"allowed": "all"},
        "limits": {"max_turns": 50, "max_output_tokens": 1_000_000}});
    let providers = issued(&broker, policy).await;
    let one_turn = config_turns(1, 1);
    let mut engine = providers
        .model(&selection())
        .unwrap()
        .factory()
        .create(one_turn, None)
        .unwrap();
    assert!(matches!(
        tool_round(engine.as_mut()).await,
        Ok(EngineAction::Complete(_))
    ));
    assert_eq!(broker.bodies().len(), 1);
}

#[tokio::test]
async fn spend_ceilings_refuse_a_call_that_would_pass_the_run_or_chat_total() {
    // Each quote costs the 50_000 micro ceiling of the boot document.
    let quote = |providers: &BrokeredProviders, chat_byte: u8, run: u8| {
        let mut config = config(chat_byte);
        config.run = RunBinding::new(
            scope(),
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(1).unwrap(),
            chat(chat_byte),
            RunId::from_bytes([run; 16]),
            RunEpoch::new(1).unwrap(),
        );
        let request = ModelCallRequest::new(config.run, StepId::from_bytes([1; 16]), 100, 100);
        providers
            .model(&selection())
            .unwrap()
            .pricing()
            .quote(&request.unwrap())
            .map(|quote| quote.spend_micros())
    };
    let policy = serde_json::json!({"tools": {"allowed": "all"}, "limits": {
        "max_spend_micros_per_run": 120_000, "max_spend_micros_per_chat": 160_000}});
    let providers = providers_with("http://127.0.0.1:9", policy);
    assert_eq!(quote(&providers, 1, 1).unwrap(), 50_000);
    assert_eq!(quote(&providers, 1, 1).unwrap(), 50_000);
    assert!(matches!(
        quote(&providers, 1, 1),
        Err(InferenceError::BudgetExceeded)
    ));
    // Another run of the same chat has its own run total but shares the chat's.
    assert!(quote(&providers, 1, 2).is_ok());
    assert!(matches!(
        quote(&providers, 1, 2),
        Err(InferenceError::BudgetExceeded)
    ));
    // Another chat is unaffected; without a policy nothing is added to the quote.
    assert!(quote(&providers, 2, 1).is_ok());
    let plain = providers_with(
        "http://127.0.0.1:9",
        serde_json::json!({"tools": {"allowed": "all"}}),
    );
    for _ in 0..5 {
        assert_eq!(quote(&plain, 1, 1).unwrap(), 50_000);
    }
}
