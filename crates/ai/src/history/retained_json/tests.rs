//! Actual owners and fixed-storage sinks; no production census or allocator added.
use super::*;
use crate::{
    history::{ChatMetadata, ComposerData, ResumeDescriptor, TranscriptData},
    model::{
        controls::Presence,
        json::{
            JsonReceipt,
            deferred::{DecodeLedger, literal::LiteralBuilder},
        },
    },
};
fn chat(
    authority: &crate::model::retention::RetentionAuthority,
    entries: Vec<Entry>,
    queue: Vec<SavedPrompt>,
) -> SavedChat {
    SavedChat::from_parts(
        authority,
        "00000000-0000-4000-8000-000000000024".into(),
        ChatMetadata {
            agent_id: "original".into(),
            title: None,
            name: None,
            pinned: false,
            updated: 0,
            model: None,
        },
        ResumeDescriptor::default(),
        TranscriptData {
            entries,
            times: Vec::new(),
        },
        ComposerData {
            queue,
            ..ComposerData::default()
        },
    )
    .unwrap()
}
fn json(ledger: &DecodeLedger) -> OwnedJson {
    let mut builder = LiteralBuilder::new(ledger).unwrap();
    builder.boolean(true).unwrap();
    builder.finish().unwrap().as_owned().clone()
}
fn annotations(value: &Extensions) -> Option<Annotations> {
    Some(Annotations {
        audience: None,
        modified: None,
        priority: None,
        extensions: Some(value.clone()),
    })
}
fn text(value: &Extensions) -> Content {
    Content::Text(Text {
        text: "original".into(),
        annotations: annotations(value),
        extensions: Some(value.clone()),
    })
}
fn contents(value: &Extensions) -> [Content; 5] {
    [
        text(value),
        Content::Media(Media {
            kind: MediaKind::Image,
            encoded: "image".into(),
            mime: "image/png".into(),
            uri: None,
            annotations: annotations(value),
            extensions: Some(value.clone()),
        }),
        Content::ResourceReference(ResourceReference {
            uri: "uri".into(),
            name: "name".into(),
            title: None,
            description: None,
            mime: None,
            reported_size: None,
            annotations: annotations(value),
            extensions: Some(value.clone()),
        }),
        Content::EmbeddedResource(EmbeddedResource {
            body: ResourceBody::Text(ResourceText {
                uri: "uri".into(),
                text: "text".into(),
                mime: None,
                extensions: Some(value.clone()),
            }),
            annotations: annotations(value),
            extensions: Some(value.clone()),
        }),
        Content::EmbeddedResource(EmbeddedResource {
            body: ResourceBody::Encoded(ResourceBlob {
                uri: "uri".into(),
                encoded: "encoded".into(),
                mime: None,
                extensions: Some(value.clone()),
            }),
            annotations: annotations(value),
            extensions: Some(value.clone()),
        }),
    ]
}
fn prompt(images: Vec<Content>) -> SavedPrompt {
    SavedPrompt {
        id: 1,
        text: "queued".into(),
        images,
        attachments: Vec::new(),
    }
}
fn tool(value: &OwnedJson, extensions: &Extensions) -> ToolRecord {
    ToolRecord {
        id: crate::model::ids::ToolId::new("original"),
        title: "tool".into(),
        name: None,
        kind: ToolKind::Other,
        status: ToolStatus::Succeeded,
        output: vec![
            ToolOutput::Content {
                content: Box::new(text(extensions)),
                extensions: Some(extensions.clone()),
            },
            ToolOutput::FileDiff(FileDiff {
                path: "file".into(),
                previous: None,
                replacement: "replacement".into(),
                extensions: Some(extensions.clone()),
            }),
            ToolOutput::TerminalReference {
                id: crate::model::ids::ProviderTerminalId::new("terminal"),
                extensions: Some(extensions.clone()),
            },
        ],
        locations: vec![Location {
            path: "location".into(),
            line: None,
            extensions: Some(extensions.clone()),
        }],
        arguments: Some(value.clone()),
        result: Some(value.clone()),
        extensions: Some(extensions.clone()),
        claims: BridgeClaims::default(),
        literal: Default::default(),
        client_display: Some(crate::model::shared_display::SharedDisplay::from_untracked(
            ClientDisplay {
                version: 2,
                tool: "display".into(),
                arguments: value.clone(),
                destination: Presence::Absent,
                redacted: Presence::Absent,
                outcome: Presence::Value(DisplayOutcome::Succeeded(value.clone())),
            },
        )),
    }
}
#[test]
fn content_and_prompt_branches_emit_their_actual_extension_roots() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let extensions = Extensions::checked(&authority, serde_json::Map::new()).unwrap();
    let expected = extensions.retained_allocations()[0].unwrap();
    for (value, expected_count) in contents(&extensions).into_iter().zip([2, 2, 2, 3, 3]) {
        let document = chat(&authority, vec![Entry::Content(value)], Vec::new());
        let mut count = 0;
        visit_chat_json_allocations(&document, None, &mut |component| {
            assert!(component.same_allocation(&expected));
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, expected_count);
    }
    let document = chat(
        &authority,
        vec![
            Entry::Agent("agent".into()),
            Entry::Thought("thought".into()),
            Entry::User(contents(&extensions).into()),
        ],
        vec![prompt(contents(&extensions).into())],
    );
    let mut count = 0;
    visit_chat_json_allocations(&document, None, &mut |component| {
        assert!(component.same_allocation(&expected));
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 24);
}
#[test]
fn tool_branches_emit_original_shared_roots_and_ledger_without_changing_credit() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 16384).unwrap();
    let value = json(&ledger);
    let extensions = Extensions::checked(&authority, serde_json::Map::new()).unwrap();
    let document = chat(
        &authority,
        vec![Entry::Tool(tool(&value, &extensions))],
        vec![prompt(vec![text(&extensions)])],
    );
    let [root, metadata] = value.retained_allocations();
    let extension = extensions.retained_allocations()[0].unwrap();
    let live = ledger.live_requested_bytes();
    let mut counts = [0; 3];
    visit_chat_json_allocations(&document, None, &mut |component| {
        if component.same_allocation(&root.unwrap()) {
            counts[0] += 1;
        } else if component.same_allocation(&metadata.unwrap()) {
            counts[1] += 1;
        } else {
            assert!(component.same_allocation(&extension));
            counts[2] += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(counts, [4, 4, 9]);
    assert_eq!(ledger.live_requested_bytes(), live);
}
#[test]
fn absent_failed_and_pending_display_outcomes_add_no_json_root() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    for outcome in [
        Presence::Absent,
        Presence::Null,
        Presence::Value(DisplayOutcome::Pending),
        Presence::Value(DisplayOutcome::Failed("failure".into())),
    ] {
        let value = OwnedJson::from_untracked(&authority, serde_json::Value::Null).unwrap();
        let mut row = tool(
            &value,
            &Extensions::checked(&authority, serde_json::Map::new()).unwrap(),
        );
        row.output.clear();
        row.locations.clear();
        row.extensions = None;
        let mut display = (**row.client_display.as_ref().unwrap()).clone();
        display.outcome = outcome;
        row.client_display = Some(crate::model::shared_display::SharedDisplay::from_untracked(
            display,
        ));
        let document = chat(&authority, vec![Entry::Tool(row)], Vec::new());
        let mut count = 0;
        visit_chat_json_allocations(&document, None, &mut |_| {
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 3);
    }
    let mut row = tool(
        &OwnedJson::from_untracked(&authority, serde_json::Value::Null).unwrap(),
        &Extensions::checked(&authority, serde_json::Map::new()).unwrap(),
    );
    row.arguments = None;
    row.result = None;
    row.extensions = None;
    row.client_display = None;
    row.output.clear();
    row.locations.clear();
    let document = chat(&authority, vec![Entry::Tool(row)], Vec::new());
    let mut count = 0;
    visit_chat_json_allocations(&document, None, &mut |_| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 0);
}
#[test]
fn whole_owner_visits_all_2006_rows_instead_of_the_newest_2000_projection() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let extensions = Extensions::checked(&authority, serde_json::Map::new()).unwrap();
    let document = chat(
        &authority,
        (0..2006)
            .map(|_| {
                Entry::Content(Content::Text(Text {
                    text: "retained".into(),
                    annotations: None,
                    extensions: Some(extensions.clone()),
                }))
            })
            .collect(),
        Vec::new(),
    );
    assert_eq!(document.view().unwrap().entries().len(), 2000);
    let mut count = 0usize;
    visit_chat_json_allocations(&document, None, &mut |_| {
        count = count
            .checked_add(1)
            .ok_or(RetentionError::ArithmeticOverflow)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 2006);
}
#[test]
fn independent_ledgers_remain_distinct_after_their_explicit_handles_are_gone() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let left = DecodeLedger::new(&authority, 16384).unwrap();
    let right = DecodeLedger::new(&authority, 16384).unwrap();
    let a = json(&left);
    let b = json(&right);
    let expected_a = a.retained_allocations()[1].unwrap();
    let expected_b = b.retained_allocations()[1].unwrap();
    assert!(!expected_a.same_allocation(&expected_b));
    let document = chat(
        &authority,
        vec![
            Entry::Tool(tool(
                &a,
                &Extensions::checked(&authority, serde_json::Map::new()).unwrap(),
            )),
            Entry::Tool(tool(
                &b,
                &Extensions::checked(&authority, serde_json::Map::new()).unwrap(),
            )),
        ],
        Vec::new(),
    );
    drop(left);
    drop(right);
    let mut counts = [0; 2];
    visit_chat_json_allocations(&document, None, &mut |component| {
        if component.same_allocation(&expected_a) {
            counts[0] += 1;
        }
        if component.same_allocation(&expected_b) {
            counts[1] += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(counts, [4, 4]);
}
struct ReceiptOwner(JsonReceipt);
impl HistoryJsonRetention for ReceiptOwner {
    fn visit_json_allocations<'a>(
        &'a self,
        visit: &mut dyn FnMut(RetainedAllocation<'a>) -> Result<(), RetentionError>,
    ) -> Result<(), RetentionError> {
        for component in self.0.retained_allocations().into_iter().flatten() {
            visit(component)?;
        }
        Ok(())
    }
}
#[test]
fn auxiliary_dyn_source_borrows_real_weak_receipt_after_payload_and_ledger_drop() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let source = {
        let ledger = DecodeLedger::new(&authority, 16384).unwrap();
        let value = json(&ledger);
        ReceiptOwner(value.receipt().clone())
    };
    let document = chat(&authority, Vec::new(), Vec::new());
    let expected = source.0.retained_allocations();
    let mut observed = [None; 2];
    let mut count = 0;
    visit_chat_json_allocations(&document, Some(&source), &mut |component| {
        assert!(component.same_allocation(&expected[count].unwrap()));
        observed[count] = Some(component);
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 2);
    assert_eq!(
        observed[0].unwrap().bytes(),
        source.0.retained_identity_bytes()
    );
}
#[test]
fn filtered_extensions_share_backing_and_sink_failure_stops_without_owner_mutation() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let original = Extensions::checked(
        &authority,
        serde_json::Map::from_iter([
            (
                crate::tool_display::META_KEY.into(),
                serde_json::Value::Null,
            ),
            ("vendor".into(), serde_json::Value::Bool(true)),
        ]),
    )
    .unwrap();
    let filtered = original.clone().without_client_display();
    let document = chat(
        &authority,
        vec![Entry::Content(text(&original))],
        vec![prompt(vec![text(&filtered)])],
    );
    let expected = original.retained_allocations()[0].unwrap();
    let mut count = 0;
    let error = visit_chat_json_allocations(&document, None, &mut |component| {
        assert!(component.same_allocation(&expected));
        count += 1;
        Err(RetentionError::ForeignLedger)
    })
    .unwrap_err();
    assert!(matches!(error, RetentionError::ForeignLedger));
    assert_eq!(count, 1);
    count = 0;
    visit_chat_json_allocations(&document, None, &mut |component| {
        assert!(component.same_allocation(&expected));
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 4);
    assert!(original.get(crate::tool_display::META_KEY).is_some());
    assert!(filtered.get(crate::tool_display::META_KEY).is_none());
}
