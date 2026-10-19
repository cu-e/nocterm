//! Immutable display identity and retained allocation travel to final ToolRecord.
use super::*;
use crate::{
    history::{DecodedRow, ToolRowHeader},
    model::{
        json::deferred::literal::LiteralBuilder,
        tools::{DisplayOutcome, ToolKind, ToolStatus},
    },
    thread::Entry,
};
fn text(value: &str, ledger: &DecodeLedger) -> LiteralText {
    let mut text = LiteralText::new(ledger).unwrap();
    text.append(value).unwrap();
    text
}
fn json(ledger: &DecodeLedger) -> DecodedJson {
    let mut builder = LiteralBuilder::new(ledger).unwrap();
    builder.null().unwrap();
    builder.finish().unwrap()
}
#[test]
fn final_display_moves_same_arc_identity_into_final_tool_row_and_keeps_receipts() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let before = ledger.live_requested_bytes();
    let display = DecodedDisplay::new(
        255,
        text("literal tool", &ledger),
        json(&ledger),
        Presence::Value(text("  original destination  ", &ledger)),
        Presence::Value(false),
        Presence::Value(DecodedDisplayOutcome::Succeeded(json(&ledger))),
        &ledger,
    )
    .unwrap();
    let identity = display.as_display() as *const ClientDisplay;
    let arguments = display.as_display().arguments.as_value() as *const serde_json::Value;
    let header = ToolRowHeader::new(
        text("id", &ledger),
        text("title", &ledger),
        ToolKind::Other,
        ToolStatus::Pending,
    );
    let row = DecodedRow::tool_with_display(header, None, None, Some(display), &ledger).unwrap();
    let Entry::Tool(call) = row.as_entry() else {
        panic!("tool")
    };
    let display = call.client_display.as_ref().unwrap();
    assert_eq!(&**display as *const ClientDisplay, identity);
    assert_eq!(
        display.arguments.as_value() as *const serde_json::Value,
        arguments
    );
    assert_eq!(display.version, 255);
    assert_eq!(
        display.destination,
        Presence::Value("  original destination  ".into())
    );
    assert!(
        matches!(&display.outcome,Presence::Value(DisplayOutcome::Succeeded(value)) if value.as_value().is_null())
    );
    assert!(ledger.live_requested_bytes() > before);
    drop(row);
    assert_eq!(ledger.live_requested_bytes(), before);
}
#[test]
fn foreign_display_arguments_cannot_mint_a_receipt_in_another_construction() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let own = DecodeLedger::new(&authority, 16384).unwrap();
    let other = DecodeLedger::new(&authority, 16384).unwrap();
    let initial = own.live_requested_bytes();
    let other_initial = other.live_requested_bytes();
    let result = DecodedDisplay::new(
        0,
        text("tool", &own),
        json(&other),
        Presence::Absent,
        Presence::Null,
        Presence::Value(DecodedDisplayOutcome::Failed(text("literal failure", &own))),
        &own,
    );
    assert!(matches!(result, Err(JsonDecodeError::ForeignLedger)));
    assert_eq!(own.live_requested_bytes(), initial);
    assert_eq!(other.live_requested_bytes(), other_initial);
}

#[test]
fn public_display_json_alias_keeps_authentic_credit_after_the_aggregate_is_gone() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 16384).unwrap();
    let baseline = ledger.live_requested_bytes();
    let display = DecodedDisplay::new(
        1,
        text("tool", &ledger),
        json(&ledger),
        Presence::Null,
        Presence::Absent,
        Presence::Value(DecodedDisplayOutcome::Succeeded(json(&ledger))),
        &ledger,
    )
    .unwrap();
    let arguments = display.as_display().arguments.clone();
    let Presence::Value(DisplayOutcome::Succeeded(result)) = &display.as_display().outcome else {
        panic!("succeeded")
    };
    let result = result.clone();
    let receipt = arguments.receipt().clone();
    drop(display);
    assert!(receipt.matches(&arguments));
    assert!(arguments.as_value().is_null());
    assert!(result.as_value().is_null());
    assert_eq!(
        ledger.live_requested_bytes(),
        baseline + arguments.retained_heap_bytes() + result.retained_heap_bytes()
    );
    drop(arguments);
    assert_eq!(
        ledger.live_requested_bytes(),
        baseline + receipt.retained_identity_bytes().unwrap() + result.retained_heap_bytes()
    );
    drop(result);
    drop(receipt);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn escaped_display_keeps_its_literal_text_credit_and_weak_metadata_retires_after_payload() {
    let authority = crate::model::retention::RetentionAuthority::new(1024 * 1024).unwrap();
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let baseline = ledger.live_requested_bytes();
    let decoded = DecodedDisplay::new(
        2,
        text("original tool", &ledger),
        json(&ledger),
        Presence::Value(text("original destination", &ledger)),
        Presence::Absent,
        Presence::Value(DecodedDisplayOutcome::Failed(text(
            "original failure",
            &ledger,
        ))),
        &ledger,
    )
    .unwrap();
    let (display, token) = decoded.into_parts(&ledger).unwrap();
    drop(token);
    let identity = display.identity();
    let arguments = display.arguments.receipt().clone();
    let expected = baseline
        + crate::model::shared_display::SharedDisplay::inline_bytes()
        + display.tool.capacity()
        + display.destination.value().unwrap().capacity()
        + match display.outcome.value().unwrap() {
            DisplayOutcome::Failed(text) => text.capacity(),
            _ => panic!("failure"),
        }
        + display.arguments.retained_heap_bytes();
    let alias = display.clone();
    drop(display);
    assert_eq!(alias.tool, "original tool");
    assert_eq!(ledger.live_requested_bytes(), expected);
    drop(alias);
    let weak_json = arguments.retained_identity_bytes().unwrap();
    assert_eq!(
        ledger.live_requested_bytes(),
        baseline + crate::model::shared_display::SharedDisplay::inline_bytes() + weak_json
    );
    drop(identity);
    assert_eq!(ledger.live_requested_bytes(), baseline + weak_json);
    drop(arguments);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
