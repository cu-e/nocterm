//! Pending projections use supplied data, independent of an existing large row.
use super::*;
use crate::model::{
    content::{Content, Text},
    ids::{SessionId, ToolId},
    tools::{BridgeClaims, ToolChanges, ToolKind, ToolRecord, ToolStatus},
};
fn notice(update: SessionUpdate) -> SessionNotice {
    SessionNotice {
        session: SessionId::new("original-session"),
        update,
    }
}
#[test]
fn chunks_use_original_payload_heap_and_retained_serializer_without_envelope_tax() {
    let mut text = String::with_capacity(8192);
    text.push_str("real retained text");
    let content = Content::Text(Text {
        text,
        annotations: None,
        extensions: None,
    });
    let original = allocations::content_payload(&content)
        .unwrap()
        .max(size::retained(&content).unwrap());
    let notice = notice(SessionUpdate::AgentChunk(content));
    assert_eq!(SourcePolicy::notice(&notice).unwrap().bytes(), original);
    assert!(original >= 8192);
}
#[test]
fn patch_measures_supplied_status_without_charging_an_unrelated_complete_tool_row() {
    let patch = ToolPatch {
        id: ToolId::new("original-tool"),
        changes: ToolChanges {
            status: Some(ToolStatus::Succeeded),
            ..ToolChanges::default()
        },
        extensions: None,
        claims: BridgeClaims::default(),
        literal: Default::default(),
    };
    let expected = size::retained(&ToolStatus::Succeeded).unwrap();
    assert_eq!(
        SourcePolicy::notice(&notice(SessionUpdate::ToolChanged(patch)))
            .unwrap()
            .bytes(),
        expected
    );
    let unchanged = ToolPatch {
        id: ToolId::new("original-tool"),
        changes: ToolChanges::default(),
        extensions: None,
        claims: BridgeClaims::default(),
        literal: Default::default(),
    };
    assert_eq!(
        SourcePolicy::notice(&notice(SessionUpdate::ToolChanged(unchanged)))
            .unwrap()
            .bytes(),
        0
    );
}
#[test]
fn tool_payload_plain_reuses_original_row_heap_without_fabricated_entry_inline() {
    let row = crate::thread::Entry::Tool(ToolRecord {
        id: ToolId::new("original-tool"),
        title: "Original tool".into(),
        name: Some("literal name".into()),
        kind: ToolKind::Read,
        status: ToolStatus::Pending,
        output: Vec::with_capacity(3),
        locations: Vec::with_capacity(2),
        arguments: None,
        result: None,
        extensions: None,
        claims: BridgeClaims::default(),
        literal: Default::default(),
        client_display: None,
    });
    let plain = allocations::row(&row).unwrap() - std::mem::size_of::<crate::thread::Entry>();
    let crate::thread::Entry::Tool(tool) = row else {
        unreachable!()
    };
    assert_eq!(allocations::tool_payload(&tool).unwrap(), plain);
    let expected = plain.max(size::retained(&tool).unwrap());
    assert_eq!(
        SourcePolicy::notice(&notice(SessionUpdate::ToolStarted(tool)))
            .unwrap()
            .bytes(),
        expected
    );
}
#[test]
fn transient_control_text_does_not_gain_a_history_row_policy() {
    let notice = notice(SessionUpdate::Info(crate::model::controls::Change::Set(
        "new title".into(),
    )));
    assert_eq!(SourcePolicy::notice(&notice).unwrap().bytes(), 0);
}

#[test]
fn approval_permission_text_and_status_remain_physical_without_a_history_component_tax() {
    let request = crate::model::auth::ProviderPermission {
        session: SessionId::new("original-session"),
        tool: ToolPatch {
            id: ToolId::new("original-tool"),
            changes: ToolChanges {
                title: Some("large approval title".repeat(1000)),
                status: Some(ToolStatus::Running),
                output: Some(vec![crate::model::tools::ToolOutput::Content {
                    content: Box::new(Content::Text(Text {
                        text: "approval content".repeat(1000),
                        annotations: None,
                        extensions: None,
                    })),
                    extensions: None,
                }]),
                ..ToolChanges::default()
            },
            extensions: None,
            claims: BridgeClaims::default(),
            literal: Default::default(),
        },
        choices: Vec::new(),
        extensions: None,
    };
    assert_eq!(SourcePolicy::permission(&request).unwrap().bytes(), 0);
}
