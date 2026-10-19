//! Used RPC replies compare with actual original projection and restore merge.
use super::fixture;
use crate::managed::{initialize, results, scan::Scanner, value::Value};
use agent_client_protocol::managed::Source;
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::retention::RetentionAuthority;

#[test]
fn opened_results_preserve_actual_models_route_and_restore_object_only_id_injection() {
    for (raw, restore) in [
        (r#"{"sessionId":"001"}"#, None),
        (r#"{"sessionId":"","modes":false}"#, None),
        (
            r#"{"sessionId":"001","modes":{"currentModeId":"m","availableModes":[false,{"id":"m","name":"Mode"}]},"configOptions":false}"#,
            None,
        ),
        (
            r#"{"sessionId":"001","modes":{"currentModeId":"m"},"configOptions":[false,{"id":"b","name":"Boolean","type":"boolean","currentValue":true}]}"#,
            None,
        ),
        (
            r#"{"sessionId":false,"configOptions":[]}"#,
            Some(" original-request "),
        ),
        (r#"["positional",null,null,null]"#, Some("original-request")),
        (r#"null"#, Some("original-request")),
    ] {
        let fixture = fixture();
        let source = Source::copy(raw, fixture.budget.clone()).unwrap();
        Scanner::new(raw, fixture.budget.clone())
            .complete()
            .unwrap();
        let native = results::opened(Value::new(&source, 0..raw.len(), &fixture.budget), restore);
        let mut original = serde_json::from_str::<serde_json::Value>(raw).unwrap();
        if let Some(id) = restore
            && let Some(object) = original.as_object_mut()
        {
            object.insert("sessionId".into(), serde_json::Value::String(id.into()));
        }
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let original = crate::models::Models::default()
            .session(original)
            .ok()
            .and_then(|value| crate::mapping::session::opened(&authority, value).ok());
        match (native, original) {
            (Ok(native), Some(original)) => {
                assert_eq!(native.get(), &original, "source:{raw},restore:{restore:?}")
            }
            (Err(_), None) => {}
            (Ok(_), None) => panic!("native accepted original rejected:{raw}"),
            (Err(error), Some(_)) => panic!("native rejected original accepted:{raw}: {error}"),
        }
    }
}
#[test]
fn prompt_results_preserve_stop_reason_and_optional_usage_defaults() {
    for raw in [
        r#"{"stopReason":"end_turn","usage":{"totalTokens":20,"inputTokens":8,"outputTokens":12,"thoughtTokens":"bad","cachedReadTokens":2,"cachedWriteTokens":3}}"#,
        r#"["max_tokens",[20,8,12,null,null,null,null],null]"#,
        r#"{"stopReason":"max_turn_requests","usage":{"totalTokens":1.0,"inputTokens":8,"outputTokens":12}}"#,
        r#"{"stopReason":"refusal","usage":false}"#,
        r#"{"stopReason":"cancelled"}"#,
        r#"{"stopReason":"future"}"#,
    ] {
        let fixture = fixture();
        let source = Source::copy(raw, fixture.budget.clone()).unwrap();
        let native = results::outcome(Value::new(&source, 0..raw.len(), &fixture.budget));
        let original =
            serde_json::from_value::<wire::PromptResponse>(serde_json::from_str(raw).unwrap())
                .ok()
                .and_then(|value| crate::mapping::session::outcome(value).ok());
        match (native, original) {
            (Ok(native), Some(original)) => {
                assert_eq!(native.get(), &original, "source:{raw}")
            }
            (Err(_), None) => {}
            (Ok(_), None) => panic!("native accepted original rejected:{raw}"),
            (Err(error), Some(_)) => panic!("native rejected original accepted:{raw}: {error}"),
        }
    }
}
#[test]
fn initialization_preserves_original_defaults_capability_presence_and_auth_fallback() {
    for raw in [
        r#"{"protocolVersion":1}"#,
        r#"{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"promptCapabilities":{"image":true,"audio":"bad","embeddedContext":true},"sessionCapabilities":{"resume":{},"fork":{},"close":[]}},"authMethods":[false,{"id":"a","name":"Agent"}],"agentInfo":{"name":"original","title":"ignored","version":"v"}}"#,
        r#"[1,[true,[true,false,true,null],null,[null,null,null,{},null,true,null],null,null],[],["original",null,"v",null],null]"#,
        r#"{"protocolVersion":2,"agentCapabilities":false,"agentInfo":{"name":"missing version"},"authMethods":false}"#,
        r#"{"protocolVersion":1.0}"#,
    ] {
        let fixture = fixture();
        let source = Source::copy(raw, fixture.budget.clone()).unwrap();
        let native = initialize::initialized(
            Value::new(&source, 0..raw.len(), &fixture.budget),
            "fallback",
        );
        let original =
            serde_json::from_value::<wire::InitializeResponse>(serde_json::from_str(raw).unwrap())
                .ok();
        match (native, original) {
            (Ok(native), Some(original)) => {
                let expected = nocterm_ai::AgentInfo {
                    name: original
                        .agent_info
                        .as_ref()
                        .map_or_else(|| "fallback".to_owned(), |value| value.name.clone()),
                    version: original
                        .agent_info
                        .map_or_else(String::new, |value| value.version),
                    capabilities: crate::mapping::session::capabilities(
                        &original.agent_capabilities,
                    ),
                    auth_methods: original
                        .auth_methods
                        .into_iter()
                        .map(crate::mapping::auth::method)
                        .collect::<Result<_, _>>()
                        .unwrap(),
                };
                assert_eq!(native.info().name, expected.name, "source:{raw}");
                assert_eq!(native.info().version, expected.version, "source:{raw}");
                assert_eq!(
                    native.info().capabilities,
                    expected.capabilities,
                    "source:{raw}"
                );
                assert_eq!(
                    native.info().auth_methods,
                    expected.auth_methods,
                    "source:{raw}"
                );
            }
            (Err(_), None) => {}
            (Ok(_), None) => panic!("native accepted original rejected:{raw}"),
            (Err(error), Some(_)) => panic!("native rejected original accepted:{raw}: {error}"),
        }
    }
}
