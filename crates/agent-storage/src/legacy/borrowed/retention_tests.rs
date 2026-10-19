//! Authentic adapter grammar participates through the pure borrowed capability.
use super::*;
use nocterm_ai::{
    history::{
        ChatMetadata, ComposerData, HistoryJsonRetention, ResumeDescriptor, RetentionError,
        SavedChat, TranscriptData, visit_chat_json_allocations,
    },
    model::{
        content::{Content, Text},
        tools::ToolRecord,
    },
};
use serde_json::json;
fn row(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
) -> (ToolRecord, super::super::order::RowOrder) {
    let bytes = serde_json::to_vec(&json!({
        "toolCallId":"original", "title":"original", "_meta":{
            "vendor":{"kept":true},
            "nocterm/tool-display":{"version":2,"tool":"original","arguments":{}}
        }
    }))
    .unwrap();
    let private = serde_json::from_slice(&bytes).unwrap();
    let (row, order) = super::super::tools::moving_ordered(authority, private).unwrap();
    (row, order.unwrap())
}
fn chat(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
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
#[test]
fn actual_grammar_row_and_prompt_emit_the_same_retained_extension_backing() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let (row, order) = row(&authority);
    let extensions = row.extensions.as_ref().unwrap().clone();
    let identity = extensions.identity();
    let expected = identity.retained_allocations()[0].unwrap();
    let mut grammar = Grammar::new(&authority);
    grammar.record(Some(order)).unwrap();
    let document = chat(
        &authority,
        vec![Entry::Tool(row)],
        vec![SavedPrompt {
            id: 1,
            text: "queued".into(),
            attachments: Vec::new(),
            images: vec![Content::Text(Text {
                text: "kept".into(),
                annotations: None,
                extensions: Some(extensions),
            })],
        }],
    );
    let mut matches = 0;
    let mut others = 0;
    visit_chat_json_allocations(&document, Some(&grammar), &mut |component| {
        if component.same_allocation(&expected) {
            matches += 1;
        } else {
            others += 1;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(
        matches, 3,
        "row, queued image and original grammar keep one backing"
    );
    assert_eq!(
        others, 1,
        "the real client display retains its arguments separately"
    );
}
#[test]
fn actual_grammar_visits_all_2006_weak_orders_after_the_strong_row_is_dropped() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let (row, order) = row(&authority);
    let identity = row.extensions.as_ref().unwrap().identity();
    let mut grammar = Grammar::new(&authority);
    grammar.record(Some(order)).unwrap();
    for _ in 1..2006 {
        grammar
            .record(super::super::order::capture(
                &row,
                0,
                super::super::order::FieldOrder::NATIVE,
            ))
            .unwrap();
    }
    drop(row);
    let expected = identity.retained_allocations()[0].unwrap();
    let source: &dyn HistoryJsonRetention = &grammar;
    let document = chat(&authority, Vec::new(), Vec::new());
    let mut count = 0usize;
    let mut last = None;
    visit_chat_json_allocations(&document, Some(source), &mut |component| {
        assert!(component.same_allocation(&expected));
        assert_eq!(component.bytes(), identity.retained_heap_bytes());
        count = count
            .checked_add(1)
            .ok_or(RetentionError::ArithmeticOverflow)?;
        last = Some(component);
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 2006);
    assert!(last.unwrap().same_allocation(&expected));
}
#[test]
fn real_grammar_propagates_sink_refusal_without_changing_original_order() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let (row, order) = row(&authority);
    let mut grammar = Grammar::new(&authority);
    grammar.record(Some(order)).unwrap();
    grammar
        .record(super::super::order::capture(
            &row,
            0,
            super::super::order::FieldOrder::NATIVE,
        ))
        .unwrap();
    let mut calls = 0;
    let error = grammar
        .visit_json_allocations(&mut |_| {
            calls += 1;
            Err(RetentionError::Limit {
                requested: 17,
                available: 16,
            })
        })
        .unwrap_err();
    assert!(matches!(
        error,
        RetentionError::Limit {
            requested: 17,
            available: 16
        }
    ));
    assert_eq!(calls, 1);
    assert!(grammar.order(&row).is_some());
    calls = 0;
    grammar
        .visit_json_allocations(&mut |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(calls, 2);
}

#[test]
fn prepaid_grammar_rows_refuse_growth_and_keep_header_credit_until_last_alias_retires() {
    use nocterm_ai::model::json::deferred::{DecodeLedger, JsonDecodeError};
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(1024 * 1024).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let (row, order) = row(&authority);
    let mut grammar = Grammar::admitted(&ledger, 1).unwrap();
    grammar.record_admitted(Some(order)).unwrap();
    assert!(matches!(
        grammar.record_admitted(super::super::order::capture(
            &row,
            0,
            super::super::order::FieldOrder::NATIVE
        )),
        Err(JsonDecodeError::ConstructionOrder)
    ));
    assert!(grammar.order(&row).is_some());
    let grammar = grammar.retained_admitted(&ledger).unwrap();
    let alias = grammar.clone();
    let with_grammar = ledger.live_requested_bytes();
    assert!(with_grammar > baseline);
    drop(grammar);
    assert_eq!(ledger.live_requested_bytes(), with_grammar);
    drop(alias);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
