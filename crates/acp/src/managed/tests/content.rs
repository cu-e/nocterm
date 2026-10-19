//! Controlled content projection follows actual SDK defaults and skipped items.
use super::fixture;
use crate::managed::{content, value::Value};
use agent_client_protocol::managed::Source;
use nocterm_ai::model::retention::RetentionAuthority;

#[test]
fn content_preserves_optional_defaults_and_original_neutral_values() {
    for raw in [
        r#"{"type":"text","text":"hello\n😀","_meta":{"reserved":{"__raw_json":"ordinary"},"x":1,"x":2}}"#,
        r#"{"type":"text","text":"x","annotations":{"audience":["user",false,"assistant","future"],"priority":"bad","lastModified":8},"_meta":true}"#,
        r#"{"type":"text","text":"x","annotations":[["assistant"],"time",0.25,null]}"#,
        r#"{"type":"text","text":"x","annotations":[null,null,null,null,5]}"#,
        r#"{"type":"image","data":"FULL","mimeType":"image/png","uri":9,"annotations":false}"#,
        r#"{"type":"audio","data":"sound","mimeType":"audio/wav","uri":"ignored"}"#,
        r#"{"type":"resource_link","name":"file","uri":"file:///a","size":"bad","title":7}"#,
        r#"{"type":"resource","resource":{"uri":"file:///a","text":"text","blob":"ignored"}}"#,
        r#"{"type":"resource","resource":{"uri":"file:///a","text":42,"blob":"encoded"}}"#,
        r#"{"type":"resource","resource":[null,"text","file:///a",null]}"#,
        r#"{"type":"resource","resource":["encoded",null,"file:///a",null]}"#,
        r#"{"type":"text","text":1}"#,
        r#"{"type":"future","text":"x"}"#,
    ] {
        let fixture = fixture();
        let source = Source::copy(raw, fixture.budget.clone()).unwrap();
        let native = content::content(Value::new(&source, 0..raw.len(), &fixture.budget));
        let authority = RetentionAuthority::new(usize::MAX).unwrap();
        let wire = serde_json::from_value::<agent_client_protocol_schema::v1::SessionNotification>(
            serde_json::json!({"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":serde_json::from_str::<serde_json::Value>(raw).unwrap()}}),
        );
        let original = wire
            .ok()
            .and_then(|wire| crate::mapping::notice::inbound(&authority, wire).ok())
            .flatten()
            .and_then(|notice| match notice.update {
                nocterm_ai::model::notice::SessionUpdate::AgentChunk(content) => Some(content),
                _ => None,
            });
        match (native, original) {
            (Ok(native), Some(original)) => {
                assert_eq!(native.get(), &original, "source:{raw}")
            }
            (Err(_), None) => {}
            (Ok(_), None) => panic!("native accepted rejected original:{raw}"),
            (Err(error), Some(_)) => panic!("native rejected original:{raw}: {error}"),
        }
    }
}
