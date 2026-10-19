//! Authentication/permission keep actual SDK fallback and strict-list semantics.
use super::fixture;
use crate::managed::{authentication, scan::Scanner, value::Value};
use agent_client_protocol::managed::Source;
use agent_client_protocol_schema::v1 as wire;

#[test]
fn controlled_authentication_matches_terminal_defaults_environment_winners_and_agent_fallback() {
    for raw in [
        r#"{"id":"001","name":"Agent","description":7}"#,
        r#"{"type":"future","id":"001","name":"Future"}"#,
        r#"{"type":false,"id":"001","name":"Fallback"}"#,
        r#"["001","positional",null,null]"#,
        r#"{"type":"terminal","id":" 001 ","name":"Terminal","args":["a",false,"é"],"env":{"Z":"z","é":"first","\u00e9":"last","A":"a"}}"#,
        r#"{"type":"terminal","id":"001","name":"Terminal","args":false,"env":{"valid":"value","bad":false}}"#,
        r#"{"type":"terminal","id":"001","name":"Terminal","env":{"bad":false,"bad":"winner"}}"#,
        r#"{"type":"terminal","id":"001","name":"Terminal","env":{"bad":"value","bad":false}}"#,
        r#"{"type":"terminal","id":"001","name":"Terminal","env":null}"#,
        r#"{"type":"terminal","id":null,"name":"Invalid"}"#,
    ] {
        let fixture = fixture();
        let source = Source::copy(raw, fixture.budget.clone()).unwrap();
        Scanner::new(raw, fixture.budget.clone())
            .complete()
            .unwrap();
        let native = authentication::method(Value::new(&source, 0..raw.len(), &fixture.budget));
        let original =
            serde_json::from_value::<wire::AuthMethod>(serde_json::from_str(raw).unwrap())
                .ok()
                .and_then(|value| crate::mapping::auth::method(value).ok());
        match (native, original) {
            (Ok(native), Some(original)) => {
                assert_eq!(native.get(), &original, "source:{raw}")
            }
            (Err(_), None) => {}
            (Ok(_), None) => panic!("native accepted rejected source:{raw}"),
            (Err(error), Some(_)) => panic!("native rejected accepted source:{raw}: {error}"),
        }
    }
}
#[test]
fn controlled_permission_preserves_strict_options_original_ids_and_metadata() {
    for raw in [
        r#"{"sessionId":" 001 ","toolCall":{"toolCallId":"tool"},"options":[{"optionId":" 0 ","name":"Allow","kind":"allow_once"},{"optionId":"2","name":"Remember","kind":"reject_always"}],"_meta":{"kept":true}}"#,
        r#"{"sessionId":"001","toolCall":{"toolCallId":"tool","title":"name","rawInput":{"a":1,"a":2}},"options":[["001","Allow","allow_always",null]]}"#,
        r#"{"sessionId":"001","toolCall":{"toolCallId":"tool"},"options":[false,{"optionId":"a","name":"Allow","kind":"allow_once"}]}"#,
        r#"{"sessionId":"001","toolCall":{"toolCallId":"tool"},"options":[{"optionId":"a","name":"Unknown","kind":"future"}]}"#,
        r#"{"sessionId":"001","toolCall":{"toolCallId":"tool"},"options":null}"#,
    ] {
        let fixture = fixture();
        let source = Source::copy(raw, fixture.budget.clone()).unwrap();
        Scanner::new(raw, fixture.budget.clone())
            .complete()
            .unwrap();
        let native = authentication::permission(&source.full(), &fixture.budget);
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let original = serde_json::from_value::<wire::RequestPermissionRequest>(
            serde_json::from_str(raw).unwrap(),
        )
        .ok()
        .and_then(|value| crate::mapping::auth::permission(&authority, value).ok());
        match (native, original) {
            (Ok(native), Some(original)) => assert_eq!(
                serde_json::to_value(native.get()).unwrap(),
                serde_json::to_value(original).unwrap(),
                "source:{raw}"
            ),
            (Err(_), None) => {}
            (Ok(_), None) => panic!("native accepted rejected source:{raw}"),
            (Err(error), Some(_)) => panic!("native rejected accepted source:{raw}: {error}"),
        }
    }
}
