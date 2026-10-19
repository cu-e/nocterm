//! Original model-cache actions compare with independently normalized SDK replies.
use super::fixture;
use crate::managed::{controls, models::Models, results, value::Value};
use agent_client_protocol::managed::Source;
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::retention::RetentionAuthority;
fn projected(
    values: Vec<wire::SessionConfigOption>,
) -> Vec<nocterm_ai::model::controls::ConfigOption> {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    values
        .into_iter()
        .map(|value| crate::mapping::controls::option(&authority, value).unwrap())
        .collect()
}
#[test]
fn legacy_session_controls_match_original_validity_duplicates_and_collision_rules() {
    for raw in [
        r#"{"sessionId":"s","models":{"currentModelId":"a","availableModels":[{"modelId":"a","name":"A"},{"modelId":"b","name":"B"}]}}"#,
        r#"{"sessionId":"s","models":{"currentModelId":"a","availableModels":[{"modelId":"a","name":" "},{"modelId":"a","name":"First valid"},{"modelId":"\u0061","name":"Discarded duplicate"}]}}"#,
        r#"{"sessionId":"s","models":{"currentModelId":"a","availableModels":[["a","Array ignored"],{"modelId":"a","name":"  Ω  "}]},"configOptions":[{"id":"__nocterm_legacy_model","name":"Other","type":"boolean","currentValue":true}]}"#,
        r#"{"sessionId":"s","models":{"currentModelId":"a","availableModels":[{"modelId":"a","name":"A"}]},"configOptions":[{"id":"model","category":"model","name":"Native","type":"boolean","currentValue":true}]}"#,
        r#"{"sessionId":"s","models":{"currentModelId":"missing","availableModels":[{"modelId":"a","name":"A"}]}}"#,
    ] {
        let fixture = fixture();
        let mut native = Models::new(fixture.budget.clone());
        let source = Source::copy(raw, fixture.budget.clone()).unwrap();
        let opened = results::opened_with_models(
            Value::new(&source, 0..raw.len(), &fixture.budget),
            None,
            Some(&mut native),
        )
        .unwrap();
        let mut original = crate::models::Models::default();
        let expected = original
            .session(serde_json::from_str(raw).unwrap())
            .unwrap();
        assert_eq!(
            opened.get().configuration,
            expected.config_options.clone().map(projected),
            "{raw}"
        );
        // Real cache keys/configuration copies survive the physical RPC line.
        drop(source);
        let empty = nocterm_ai::model::native::NativeList::new(fixture.budget.ledger(), 0)
            .unwrap()
            .finish();
        let retained = native.retain("s", empty).unwrap();
        assert_eq!(
            retained.get(),
            &projected(original.retain(&expected.session_id, Vec::new())),
            "{raw}"
        );
    }
}
#[test]
fn original_cache_selection_changes_only_after_success_and_preserves_delivered_copy() {
    let fixture = fixture();
    let raw = r#"{"sessionId":" s Ω ","models":{"currentModelId":"a","availableModels":[{"modelId":"a","name":"A"},{"modelId":"b","name":"B"}]}}"#;
    let source = Source::copy(raw, fixture.budget.clone()).unwrap();
    let mut native = Models::new(fixture.budget.clone());
    let opened = results::opened_with_models(
        Value::new(&source, 0..raw.len(), &fixture.budget),
        None,
        Some(&mut native),
    )
    .unwrap();
    let id = opened.get().configuration.as_ref().unwrap()[0].id.as_str();
    let mut original = crate::models::Models::default();
    original
        .session(serde_json::from_str(raw).unwrap())
        .unwrap();
    let request = wire::SetSessionConfigOptionRequest::new(" s Ω ", id.to_owned(), "b");
    assert_eq!(
        native
            .selection(" s Ω ", id, Some("b"))
            .unwrap()
            .unwrap()
            .as_str(),
        original.selection(&request).unwrap().unwrap()
    );
    assert!(native.selection(" s Ω ", id, None).is_err());
    assert!(native.selection(" s Ω ", id, Some("missing")).is_err());
    let updated = native.selected(" s Ω ", Some("b")).unwrap();
    assert_eq!(updated.get(), &projected(original.selected(&request)));
    // Old delivered configuration retains its original selected value.
    let nocterm_ai::model::controls::ConfigKind::Select { current, .. } =
        &opened.get().configuration.as_ref().unwrap()[0].kind
    else {
        panic!("legacy select")
    };
    assert_eq!(current.as_str(), "a");
    let raw = r#"[{"id":"boolean","type":"boolean","name":"Boolean","currentValue":true}]"#;
    let source = Source::copy(raw, fixture.budget.clone()).unwrap();
    let incoming = controls::options(Value::new(&source, 0..raw.len(), &fixture.budget)).unwrap();
    let delivered = native.retain(" s Ω ", incoming).unwrap();
    let original_options = serde_json::from_str(raw).unwrap();
    assert_eq!(
        delivered.get(),
        &projected(original.retain(&wire::SessionId::from(" s Ω "), original_options))
    );
    native.remove(" s Ω ");
    assert!(native.selection(" s Ω ", id, Some("a")).unwrap().is_none());
}
