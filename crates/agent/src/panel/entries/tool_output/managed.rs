//! The production row renderer preserves literal tool sections through borrowed physical slots.
use super::*;
use nocterm_ai::{
    history::physical::RowBody,
    model::{
        content::{Media, MediaKind, Text},
        ids::{ProviderTerminalId, ToolId},
        tools::{FileDiff, ToolKind, ToolStatus},
    },
    testing::ManagedRows,
    thread::Entry,
};

fn call() -> ToolRecord {
    ToolRecord {
        id: ToolId::new("tool"),
        title: "Literal\nscript".into(),
        name: None,
        kind: ToolKind::Other,
        status: ToolStatus::Succeeded,
        output: vec![
            ToolOutput::Content {
                content: Box::new(Content::Text(Text {
                    text: "literal\n```output```".into(),
                    annotations: None,
                    extensions: None,
                })),
                extensions: None,
            },
            ToolOutput::Content {
                content: Box::new(Content::Media(Media {
                    kind: MediaKind::Image,
                    encoded: "AQID".into(),
                    mime: "image/png".into(),
                    uri: Some("image://original".into()),
                    annotations: None,
                    extensions: None,
                })),
                extensions: None,
            },
            ToolOutput::FileDiff(FileDiff {
                path: "file.txt".into(),
                previous: Some("old".into()),
                replacement: "new".into(),
                extensions: None,
            }),
            ToolOutput::TerminalReference {
                id: ProviderTerminalId::new("terminal"),
                extensions: None,
            },
        ],
        locations: Vec::new(),
        arguments: None,
        result: None,
        extensions: None,
        claims: Default::default(),
        literal: Default::default(),
        client_display: None,
    }
}

fn assert_output(expected: Output, actual: Output) {
    assert_eq!(expected.parameters, actual.parameters);
    assert_eq!(expected.sections.len(), actual.sections.len());
    for (expected, actual) in expected.sections.iter().zip(&actual.sections) {
        assert_eq!(expected.label, actual.label);
        assert_eq!(expected.language, actual.language);
        assert_eq!(expected.text, actual.text);
    }
}

#[test]
fn managed_tool_rows_preserve_literal_sections_and_full_content_wire() {
    let call = call();
    let rows = ManagedRows::new(vec![Entry::Tool(call.clone())]).unwrap();
    let RowBody::Tool(managed) = rows.rows()[0].view() else {
        panic!("expected physical tool");
    };
    assert_eq!(
        tool_display::header(&call),
        tool_display::managed_header(managed)
    );
    let expected = super::super::tool_input::source(&call).unwrap();
    let actual = super::super::tool_input::source_managed(managed).unwrap();
    assert_eq!(expected.label, actual.label);
    assert_eq!(expected.text, actual.text);
    assert_eq!(expected.parameters, actual.parameters);
    assert_eq!(expected.language, actual.language);
    for (original, physical) in call.output.iter().zip(managed.output()) {
        assert_eq!(content_value(original), managed_content_value(physical));
    }
    for redact in [false, true] {
        assert_output(
            source(&call, redact).unwrap(),
            source_managed(managed, redact).unwrap(),
        );
    }
}

#[test]
fn managed_tool_display_keeps_requested_parameters_and_original_failure_outcome() {
    use nocterm_ai::model::{json::OwnedJson, retention::RetentionAuthority};
    use serde_json::json;
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let mut call = call();
    call.arguments =
        Some(OwnedJson::from_untracked(&authority, json!({"program":"provider value"})).unwrap());
    let mut display = ToolDisplay::requested(
        &authority,
        "exec_command".into(),
        json!({"program":false,"extra":{"field":"literal"}}),
    )
    .unwrap();
    display.finish(Err("Invalid arguments".into())).unwrap();
    display.attach(&mut call);
    let rows = ManagedRows::with_authority(&authority, vec![Entry::Tool(call.clone())]).unwrap();
    let RowBody::Tool(managed) = rows.rows()[0].view() else {
        panic!("physical tool");
    };
    assert_eq!(
        tool_display::header(&call),
        tool_display::managed_header(managed)
    );
    let input = super::super::tool_input::source_managed(managed).unwrap();
    assert_eq!(input.label, "Requested parameters");
    assert!(input.text.contains("program: false"));
    assert!(input.text.contains("field: literal"));
    assert!(!input.text.contains("provider value"));
    assert_output(
        source(&call, false).unwrap(),
        source_managed(managed, false).unwrap(),
    );
    let output = source_managed(managed, false).unwrap();
    assert_eq!(output.sections.len(), 1);
    assert_eq!(output.sections[0].label, "Error");
    assert_eq!(output.sections[0].text, "Invalid arguments");
}
