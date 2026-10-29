use super::{fixtures::*, *};
use nocterm_ai::{ExecCommand, TerminalCall};
use nocterm_cloud_model::{
    engine::{
        EngineAction, EngineHistory, EngineHistoryMessage, HistoricalAssistant, HistoricalToolCall,
        HistoricalToolResult,
    },
    identity::IntentId,
    payload::BoundedJson,
};
use serde_json::json;

fn history() -> EngineHistory {
    let intent = IntentId::from_bytes([9; 16]);
    let call = HistoricalToolCall::new(
        intent,
        TerminalCall::ExecCommand(ExecCommand {
            terminal_id: "old-session".into(),
            program: "printf".into(),
            args: vec!["first".into()],
            stdin: None,
            timeout_ms: None,
            yield_ms: None,
        }),
    )
    .unwrap();
    EngineHistory::new(vec![
        EngineHistoryMessage::User(EnginePrompt::new(vec!["first question".into()]).unwrap()),
        EngineHistoryMessage::Assistant(
            HistoricalAssistant::new("checking".into(), vec![call]).unwrap(),
        ),
        EngineHistoryMessage::ToolResult(HistoricalToolResult::new(
            intent,
            BoundedJson::new(json!({"stdout":"first"})).unwrap(),
            false,
        )),
        EngineHistoryMessage::Assistant(
            HistoricalAssistant::new("first answer".into(), Vec::new()).unwrap(),
        ),
    ])
    .unwrap()
}
#[tokio::test]
async fn new_user_prompt_sees_authoritative_prior_exchange_and_paired_tool_history() {
    for kind in [ProviderKind::OpenAiChat, ProviderKind::AnthropicMessages] {
        let body = match kind {
            ProviderKind::OpenAiChat => chat_text("second answer", true),
            ProviderKind::AnthropicMessages => anthropic(
                json!({"type":"text","text":""}),
                json!({"type":"text_delta","text":"second answer"}),
                "end_turn",
            ),
        };
        let server = Server::start(vec![http(&body)]).await;
        let factory = factory(kind, server.root());
        let mut config = config();
        config.history = history();
        let mut engine = factory.create(config, None).unwrap();
        assert!(matches!(
            advance(engine.as_mut()).await.unwrap(),
            EngineAction::Complete(_)
        ));
        let requests = server.requests();
        let wire = requests[0].1.to_string();
        assert!(wire.contains("first question"));
        assert!(wire.contains("first answer"));
        assert!(wire.contains("hello"));
        assert!(wire.contains("nocterm_09090909090909090909090909090909"));
        if kind == ProviderKind::AnthropicMessages {
            assert_eq!(requests[0].1["thinking"]["type"], "disabled");
            assert!(wire.contains("tool_result"));
        } else {
            let messages = requests[0].1["messages"].as_array().unwrap();
            assert_eq!(messages.iter().filter(|m| m["role"] == "tool").count(), 1);
        }
    }
}
#[tokio::test]
async fn changed_seed_history_cannot_be_substituted_during_exact_run_restore() {
    let server = Server::start(vec![http(&chat_text("done", true))]).await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let mut original = config();
    original.history = history();
    let mut engine = factory.create(original, None).unwrap();
    advance(engine.as_mut()).await.unwrap();
    let checkpoint = engine.checkpoint(Default::default()).unwrap();
    assert!(factory.create(config(), Some(checkpoint)).is_err());
}
#[tokio::test]
async fn complete_context_budget_is_checked_before_opening_provider_http() {
    let server = Server::start(vec![]).await;
    let factory = factory(ProviderKind::OpenAiChat, server.root());
    let mut config = config();
    config.history = history();
    config.budget = ModelBudget::new(4, 1, 1024, 1024).unwrap();
    assert!(matches!(
        factory.create(config, None),
        Err(EngineError::BudgetExceeded)
    ));
    assert!(server.requests().is_empty());
}
#[tokio::test]
async fn signed_thinking_continuation_is_explicitly_refused_before_tools() {
    let body = anthropic(
        json!({"type":"thinking","thinking":"","signature":"signed"}),
        json!({"type":"thinking_delta","thinking":"private reasoning"}),
        "end_turn",
    );
    let server = Server::start(vec![http(&body)]).await;
    let mut engine = engine(ProviderKind::AnthropicMessages, server.root());
    assert!(matches!(
        advance(engine.as_mut()).await,
        Err(EngineError::UnsupportedCapability(_))
    ));
}


fn completed_tool_history() -> EngineHistory {
    let mut messages = history().messages().to_vec();
    messages.pop();
    EngineHistory::new(messages).unwrap()
}

#[tokio::test]
async fn authoritative_tool_history_continues_without_a_synthetic_user_message() {
    for kind in [ProviderKind::OpenAiChat, ProviderKind::AnthropicMessages] {
        let body = match kind {
            ProviderKind::OpenAiChat => chat_text("continued", true),
            ProviderKind::AnthropicMessages => anthropic(json!({"type":"text","text":""}), json!({"type":"text_delta","text":"continued"}), "end_turn"),
        };
        let server = Server::start(vec![http(&body)]).await;
        let mut config = config();
        config.history = completed_tool_history();
        let mut engine = factory(kind, server.root()).create(config, None).unwrap();
        let (_, registration) = AbortHandle::new_pair();
        assert!(matches!(engine.advance(EngineInput {next_step: StepId::from_bytes([1;16]), request: EngineRequest::ContinueHistory}, Arc::new(Events::default()), admission(), registration).await.unwrap(), EngineAction::Complete(_)));
        let id = "nocterm_09090909090909090909090909090909";
        let seed = completed_tool_history();
        let EngineHistoryMessage::Assistant(assistant) = &seed.messages()[1] else {panic!("assistant")};
        let arguments = assistant.calls()[0].arguments().value().clone();
        let requests = server.requests();
        let mut messages = requests[0].1["messages"].clone();
        let expected = match kind {
            ProviderKind::OpenAiChat => {
                let encoded = messages[1]["tool_calls"][0]["function"]["arguments"].as_str().unwrap();
                let decoded = serde_json::from_str(encoded).unwrap();
                messages[1]["tool_calls"][0]["function"]["arguments"] = decoded;
                json!([
                    {"role":"user","content":"first question"},
                    {"role":"assistant","content":"checking","tool_calls":[{"id":id,"type":"function","function":{"name":"exec_command","arguments":arguments}}]},
                    {"role":"tool","tool_call_id":id,"content":"{\"stdout\":\"first\"}"}
                ])
            }
            ProviderKind::AnthropicMessages => json!([
                {"role":"user","content":[{"type":"text","text":"first question"}]},
                {"role":"assistant","content":[{"type":"text","text":"checking"},{"type":"tool_use","id":id,"name":"exec_command","input":arguments}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":[{"type":"text","text":"{\"stdout\":\"first\"}"}]}]}
            ]),
        };
        assert_eq!(messages, expected);
    }
}

#[tokio::test]
async fn continuation_refuses_empty_completed_text_or_unfinished_tool_history_before_http() {
    let server = Server::start(vec![]).await;
    for seed in [EngineHistory::default(), history()] {
        let mut config = config();
        config.history = seed;
        let mut engine = factory(ProviderKind::OpenAiChat, server.root()).create(config, None).unwrap();
        let (_, registration) = AbortHandle::new_pair();
        assert!(matches!(engine.advance(EngineInput {next_step:StepId::from_bytes([1;16]),request:EngineRequest::ContinueHistory},Arc::new(Events::default()),admission(),registration).await,Err(EngineError::InvalidStream)));
    }
    let mut missing = completed_tool_history().messages().to_vec();
    missing.pop();
    assert!(EngineHistory::new(missing).is_err());
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn continuation_cannot_replace_a_live_pending_tool_batch() {
    let response = chat_tools(r#"{"terminal_id":"t","program":"true","args":[]}"#,1,true);
    let server = Server::start(vec![http(&response)]).await;
    let mut config = config();
    config.history = completed_tool_history();
    let mut engine = factory(ProviderKind::OpenAiChat,server.root()).create(config,None).unwrap();
    let (_, registration) = AbortHandle::new_pair();
    assert!(matches!(engine.advance(EngineInput {next_step:StepId::from_bytes([1;16]),request:EngineRequest::ContinueHistory},Arc::new(Events::default()),admission(),registration).await.unwrap(),EngineAction::Tools(_)));
    let (_, registration) = AbortHandle::new_pair();
    assert!(matches!(engine.advance(EngineInput {next_step:StepId::from_bytes([2;16]),request:EngineRequest::ContinueHistory},Arc::new(Events::default()),admission(),registration).await,Err(EngineError::InvalidStream)));
    assert_eq!(server.requests().len(),1);
}
