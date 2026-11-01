use super::{fixtures::*, *};
use nocterm_ai::model::session::StopReason;
use nocterm_cloud_model::engine::{EngineAction, TextChannel};
use serde_json::{Value, json};

#[tokio::test]
async fn openai_wire_streams_neutral_text_and_reported_usage() {
    let server = Server::start(vec![http(&chat_text("answer", true))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let events = Arc::new(Events::default());
    let (_, registration) = AbortHandle::new_pair();
    let result = engine
        .advance(input(1), events.clone(), admission(), registration)
        .await
        .unwrap();
    let EngineAction::Complete(done) = result else {
        panic!("completed");
    };
    assert_eq!(done.stop, StopReason::EndTurn);
    assert_eq!(done.usage.input_tokens, Some(11));
    assert_eq!(done.usage.output_tokens, Some(7));
    assert_eq!(done.usage.cost_micros, None);
    assert!(events.events().iter().any(|e| matches!(e, EngineEvent::Text(t) if t.text()=="answer" && t.channel()==TextChannel::Assistant)));
    let requests = server.requests();
    assert!(
        requests[0]
            .0
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {KEY}"))
    );
    assert!(requests[0].0.starts_with("POST /chat/completions "));
    assert_eq!(requests[0].1["tools"].as_array().unwrap().len(), 11);
    assert_eq!(requests[0].1["model"], "fixture-model");
}
#[tokio::test]
async fn missing_usage_stays_unknown_and_checkpoint_excludes_provider_key() {
    let server = Server::start(vec![http(&chat_text("answer", false))]).await;
    let mut engine = engine(ProviderKind::OpenAiChat, server.root());
    let EngineAction::Complete(done) = advance(engine.as_mut()).await.unwrap() else {
        panic!("complete");
    };
    assert_eq!(done.usage.total_tokens, None);
    let saved = engine
        .checkpoint(nocterm_cloud_model::identity::StreamRevision::from_u64(9))
        .unwrap();
    assert!(!String::from_utf8_lossy(saved.bytes().as_bytes()).contains(KEY));
}
#[tokio::test]
async fn anthropic_messages_wire_streams_text_and_preserves_usage() {
    let body = anthropic(
        json!({"type":"text","text":""}),
        json!({"type":"text_delta","text":"answer"}),
        "end_turn",
    );
    let server = Server::start(vec![http(&body)]).await;
    let mut engine = engine(ProviderKind::AnthropicMessages, server.root());
    let EngineAction::Complete(done) = advance(engine.as_mut()).await.unwrap() else {
        panic!("complete");
    };
    assert_eq!(done.stop, StopReason::EndTurn);
    assert_eq!(done.usage.input_tokens, Some(11));
    assert_eq!(done.usage.output_tokens, Some(7));
    let requests = server.requests();
    assert!(requests[0].0.starts_with("POST /v1/messages "));
    assert!(requests[0].0.contains(&format!("x-api-key: {KEY}")));
    assert_eq!(requests[0].1["tools"].as_array().unwrap().len(), 11);
}
#[tokio::test]
async fn anthropic_finished_tool_arguments_use_existing_terminal_catalog() {
    let args = r#"{"terminal_id":"session","program":"printf","args":["literal;$HOME"]}"#;
    let body = anthropic(
        json!({"type":"tool_use","id":"call-1","name":"exec_command","input":{}}),
        json!({"type":"input_json_delta","partial_json":args}),
        "tool_use",
    );
    let server = Server::start(vec![http(&body)]).await;
    let mut engine = engine(ProviderKind::AnthropicMessages, server.root());
    let EngineAction::Tools(batch) = advance(engine.as_mut()).await.unwrap() else {
        panic!("tools");
    };
    assert_eq!(batch.calls().len(), 1);
    assert_eq!(batch.calls()[0].call().name(), "exec_command");
    assert_eq!(batch.calls()[0].call().terminal_id(), Some("session"));
}
#[tokio::test]
async fn unfinished_or_failed_stream_never_exposes_assembled_tools() {
    let args = r#"{"terminal_id":"session","program":"printf"}"#;
    for body in [
        chat_tools(args, 1, false),
        chat_tools(args, 1, false)
            + &data(
                json!({"error":{"message":"secret echoed","type":"server_error","code":"failure"}}),
            ),
    ] {
        let server = Server::start(vec![http(&body)]).await;
        let mut engine = engine(ProviderKind::OpenAiChat, server.root());
        let error = advance(engine.as_mut()).await.unwrap_err();
        assert!(matches!(error, EngineError::Provider(_)));
        assert!(!format!("{error:?} {error}").contains("secret echoed"));
        assert!(engine.checkpoint(Default::default()).is_err());
    }
}
#[tokio::test]
async fn malformed_unknown_or_over_limit_tool_is_refused_without_effects() {
    for args in [
        "not JSON",
        r#"{"terminal_id":"session","program":""}"#,
        r#"{"terminal_id":"session","program":"printf","extra":true}"#,
    ] {
        let server = Server::start(vec![http(&chat_tools(args, 1, true))]).await;
        let mut engine = engine(ProviderKind::OpenAiChat, server.root());
        assert!(advance(engine.as_mut()).await.is_err());
    }
    let body = chat(
        json!({"tool_calls":[{"index":0,"id":"bad","type":"function","function":{"name":"shell","arguments":"{}"}}]}),
        Value::Null,
    ) + &chat(json!({}), json!("tool_calls"))
        + "data: [DONE]\n\n";
    let server = Server::start(vec![http(&body)]).await;
    assert!(
        advance(engine(ProviderKind::OpenAiChat, server.root()).as_mut())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn anthropic_missing_final_usage_is_unknown_and_unfinished_tool_or_error_is_refused() {
    let text = anthropic(
        json!({"type":"text","text":""}),
        json!({"type":"text_delta","text":"answer"}),
        "end_turn",
    );
    let missing = text.replace(r#",\"usage\":{\"output_tokens\":7}"#, "");
    // Build the terminal event explicitly: initial output_tokens=0 is not the
    // completed response's usage and must not become a fabricated final zero.
    let missing: String = missing
        .lines()
        .map(|line| {
            if line.starts_with("data: ") {
                let mut value: Value =
                    serde_json::from_str(line.trim_start_matches("data: ")).unwrap();
                if value["type"] == "message_delta" {
                    value.as_object_mut().unwrap().remove("usage");
                }
                format!("data: {value}\n")
            } else {
                "\n".into()
            }
        })
        .collect();
    let server = Server::start(vec![http(&missing)]).await;
    let mut engine = engine(ProviderKind::AnthropicMessages, server.root());
    let EngineAction::Complete(done) = advance(engine.as_mut()).await.unwrap() else {
        panic!("complete");
    };
    assert_eq!(done.usage.output_tokens, None);
    let full = anthropic(
        json!({"type":"tool_use","id":"call","name":"exec_command","input":{}}),
        json!({"type":"input_json_delta","partial_json":r#"{"terminal_id":"s","program":"printf"}"#}),
        "tool_use",
    );
    let partial: String = full
        .lines()
        .filter_map(|line| {
            if let Some(data) = line.strip_prefix("data: ") {
                let value: Value = serde_json::from_str(data).unwrap();
                if value["type"] == "message_delta" || value["type"] == "message_stop" {
                    return None;
                }
            }
            Some(format!("{line}\n"))
        })
        .collect();
    assert!(!partial.contains("\"type\":\"message_delta\""));
    for body in [
        partial.clone(),
        partial
            + &data(
                json!({"type":"error","error":{"type":"overloaded_error","message":"secret echoed"}}),
            ),
    ] {
        let server = Server::start(vec![http(&body)]).await;
        let mut native = super::engine(ProviderKind::AnthropicMessages, server.root());
        assert!(advance(native.as_mut()).await.is_err());
    }
}
#[tokio::test]
async fn terminal_reasons_are_preserved_and_a_truncated_tool_is_never_admitted() {
    for (reason, stop) in [
        ("length", StopReason::MaxTokens),
        ("content_filter", StopReason::Refusal),
    ] {
        let body = chat(json!({"content":"partial answer"}), Value::Null)
            + &chat(json!({}), json!(reason))
            + "data: [DONE]\n\n";
        let server = Server::start(vec![http(&body)]).await;
        let mut engine = engine(ProviderKind::OpenAiChat, server.root());
        let result = advance(engine.as_mut()).await;
        if reason == "content_filter" {
            assert!(matches!(result, Err(EngineError::Provider(_))));
        } else {
            let EngineAction::Complete(done) = result.unwrap() else {
                panic!("complete");
            };
            assert_eq!(done.stop, stop);
        }
        let body = chat_tools(r#"{"terminal_id":"s","program":"printf"}"#, 1, true).replace(
            "\"finish_reason\":\"tool_calls\"",
            &format!("\"finish_reason\":\"{reason}\""),
        );
        let server = Server::start(vec![http(&body)]).await;
        let mut native = super::engine(ProviderKind::OpenAiChat, server.root());
        assert!(advance(native.as_mut()).await.is_err());
    }
}
