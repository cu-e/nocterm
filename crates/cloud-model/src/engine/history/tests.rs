use super::*;
use nocterm_ai::ExecCommand;
use serde_json::json;

fn call(id: u8) -> HistoricalToolCall {
    HistoricalToolCall::new(
        IntentId::from_bytes([id; 16]),
        TerminalCall::ExecCommand(ExecCommand {
            terminal_id: "session".into(),
            program: "printf".into(),
            args: vec!["literal;$HOME".into()],
            stdin: None,
            timeout_ms: None,
            yield_ms: None,
        }),
    )
    .unwrap()
}
fn assistant(ids: &[u8]) -> EngineHistoryMessage {
    EngineHistoryMessage::Assistant(
        HistoricalAssistant::new("working".into(), ids.iter().copied().map(call).collect())
            .unwrap(),
    )
}
fn result(id: u8) -> EngineHistoryMessage {
    EngineHistoryMessage::ToolResult(HistoricalToolResult::new(
        IntentId::from_bytes([id; 16]),
        BoundedJson::new(json!({"status":"unknown; reconciliation required"})).unwrap(),
        true,
    ))
}
#[test]
fn history_preserves_pairs_without_reusing_or_inventing_intents() {
    let history = EngineHistory::new(vec![
        EngineHistoryMessage::User(EnginePrompt::new(vec!["old prompt".into()]).unwrap()),
        assistant(&[1, 2]),
        result(2),
        result(1),
    ])
    .unwrap();
    assert_eq!(history.messages().len(), 4);
    assert!(history.encoded_bytes() > 100);
    assert!(!format!("{history:?}").contains("old prompt"));
    assert!(EngineHistory::new(vec![assistant(&[1])]).is_err());
    assert!(EngineHistory::new(vec![result(1)]).is_err());
    assert!(
        EngineHistory::new(vec![assistant(&[1]), result(1), assistant(&[1]), result(1)]).is_err()
    );
    assert!(EngineHistory::new(vec![assistant(&[1, 1]), result(1)]).is_err());
    assert!(EngineHistory::new(vec![assistant(&[1]), assistant(&[2])]).is_err());
}
#[test]
fn history_bounds_aggregate_even_when_each_message_is_individually_valid() {
    let prompt = EnginePrompt::new(vec!["x".repeat(1024 * 1024).into()]).unwrap();
    let messages = (0..33)
        .map(|_| EngineHistoryMessage::User(prompt.clone()))
        .collect();
    assert!(EngineHistory::new(messages).is_err());
    assert!(HistoricalAssistant::new(String::new(), Vec::new()).is_err());
}
#[test]
fn historical_call_uses_the_shared_semantic_argument_format() {
    let historical = call(3);
    let spec = nocterm_ai::tools::catalog::get(historical.call().name()).unwrap();
    let parsed = (spec.parse)(historical.arguments().value().clone()).unwrap();
    assert_eq!(
        parsed.arguments().unwrap(),
        historical.arguments().value().clone()
    );
    assert!(!format!("{historical:?}").contains("literal"));
}
