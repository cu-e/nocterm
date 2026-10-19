//! Strict permission diagnostics compare against the original pinned SDK schema.
use super::*;
use crate::managed::tests::fixture;
#[test]
fn closed_permission_diagnostics_match_original_shapes_order_and_enum_forms() {
    for raw in [
        "null",
        "true",
        "[]",
        r#"["s"]"#,
        r#"["s",{"toolCallId":"t"}]"#,
        r#"["s",{"toolCallId":"t"},[],null,null]"#,
        r#"{}"#,
        r#"{"sessionId":false,"options":false,"toolCall":null}"#,
        r#"{"options":false,"sessionId":false,"toolCall":null}"#,
        r#"{"sessionId":"s","toolCall":[],"options":[]}"#,
        r#"{"sessionId":"s","toolCall":{},"options":[]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":3},"options":[]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[false]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{}]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"kind":"bogus","name":false}]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"name":false,"kind":"bogus"}]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"optionId":"a","name":"A","kind":1}]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"optionId":"a","name":"A","kind":{}}]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"optionId":"a","name":"A","kind":{"allow_once":false}}]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"optionId":"a","name":"A","kind":{"allow_once":null,"reject_once":null}}]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"optionId":"a","name":"A","kind":{"allow_once":null}}]}"#,
        r#"["s",{"toolCallId":"t"},[["a","A","reject_always"]]]"#,
        r#"{"sessionId":false,"options":false,"sessionId":"s","toolCall":{"toolCallId":"t"}}"#,
        r#"{"sessionId":2.5,"toolCall":{"toolCallId":"t"},"options":[]}"#,
        r#"{"sessionId":"s","toolCall":{"toolCallId":"t"},"options":[{"optionId":"a","name":"A","kind":"unknown Ω\n`"}]}"#,
    ] {
        let fixture = fixture();
        let source = Source::copy(raw, fixture.budget.clone()).unwrap();
        let span = source.full();
        let actual = diagnostic(Some(&span), &fixture.budget).unwrap();
        let value: serde_json::Value = serde_json::from_str(raw).unwrap();
        let original = agent_client_protocol::util::json_cast_params::<
            _,
            agent_client_protocol_schema::v1::RequestPermissionRequest,
        >(value.clone());
        match (actual, original) {
            (None, Ok(_)) => {
                drop(super::super::authentication::permission(&span, &fixture.budget).unwrap());
            }
            (Some(actual), Err(original)) => {
                let expected = original
                    .data
                    .as_ref()
                    .unwrap()
                    .get("error")
                    .unwrap()
                    .as_str()
                    .unwrap();
                assert_eq!(actual.text(), expected, "{raw}");
                let response = response(
                    &fixture.budget,
                    &agent_client_protocol::managed::MessageId::Number(17),
                    Some(&span),
                    &actual,
                )
                .unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(response.original().text()).unwrap(),
                    serde_json::json!({"jsonrpc":"2.0","id":17,"error":original}),
                    "{raw}"
                );
            }
            (actual, original) => panic!(
                "acceptance mismatch: actual {},original {}: {raw}",
                actual.is_none(),
                original.is_ok()
            ),
        }
    }
}
