//! Pinned old/new lexical counts and current iterative v2 validation boundaries.
use super::*;
use nocterm_ai::history::{PreflightFailure, preflight_decoder};
use std::io::Cursor;
fn former(bytes: &[u8]) -> Result<PreflightBounds, PreflightFailure<serde_json::Error>> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let bounds = preflight_decoder(&mut decoder)?;
    decoder.end().map_err(PreflightFailure::Json)?;
    Ok(bounds)
}
fn current(
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    bytes: &[u8],
) -> Result<PreflightBounds, LiteralReadError> {
    preflight(
        Cursor::new(bytes),
        &DecodeLedger::new(authority, 1024 * 1024).unwrap(),
    )
}
#[test]
fn literal_preflight_preserves_exact_original_nodes_text_and_structural_metrics() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for bytes in [
        br#"null"#.as_slice(),
        br#"true"#,
        br#"-1.25e+3"#,
        br#"{}"#,
        br#"[]"#,
        br#"{"duplicate":[1,2],"duplicate":0,"unknown":{"ignored":true}}"#,
        br#"{"$serde_json::private::RawValue":"literal\n\uD834\uDD1E"}"#,
        r#"{"日本":" escaped\u0061 ","empty":""}"#.as_bytes(),
    ] {
        assert_eq!(
            current(&authority, bytes).unwrap(),
            former(bytes).unwrap(),
            "{:?}",
            bytes
        );
    }
    let counts = current(&authority, br#"{"x":[1,2],"x":0}"#).unwrap();
    assert_eq!(
        (
            counts.nodes,
            counts.decoded_text_bytes,
            counts.structural_bytes
        ),
        (7, 2, 3588)
    );
}
#[test]
fn literal_preflight_handles_200_levels_without_building_a_retained_tree() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let source = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    assert!(former(source.as_bytes()).is_err());
    let bounds = current(&authority, source.as_bytes()).unwrap();
    assert_eq!(bounds.nodes, 201);
    assert_eq!(bounds.structural_bytes, 201 * 512);
    assert_eq!(bounds.decoded_text_bytes, 0);
}
#[test]
fn original_structural_node_limit_is_checked_before_next_current_stack_growth() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    // Flat source proves exact old/new boundary; deep source separately proves
    // checked iterative frame growth remains below supplied working allowance.
    let flat = format!("[{}0]", "0,".repeat(131070));
    let bounds = current(&authority, flat.as_bytes()).unwrap();
    assert_eq!(bounds.nodes, 131072);
    assert_eq!(bounds.structural_bytes, 64 * 1024 * 1024);
    assert_eq!(bounds, former(flat.as_bytes()).unwrap());
    let above = format!("[{}0]", "0,".repeat(131071));
    assert!(matches!(
        former(above.as_bytes()),
        Err(PreflightFailure::StructuralLimit)
    ));
    assert!(matches!(
        current(&authority, above.as_bytes()),
        Err(LiteralReadError::Preflight(
            PreflightFailure::StructuralLimit
        ))
    ));
    // Mixed N/T boundary reaches exactly64MiB without a huge String fixture.
    let mixed = format!("[{}\"{}\"]", "0,".repeat(131069), "x".repeat(256));
    let exact = current(&authority, mixed.as_bytes()).unwrap();
    assert_eq!(exact, former(mixed.as_bytes()).unwrap());
    assert_eq!(
        (
            exact.nodes,
            exact.decoded_text_bytes,
            exact.structural_bytes
        ),
        (131071, 256, 64 * 1024 * 1024)
    );
    let over_text = format!("[{}\"{}\"]", "0,".repeat(131069), "x".repeat(257));
    assert!(matches!(
        former(over_text.as_bytes()),
        Err(PreflightFailure::StructuralLimit)
    ));
    assert!(matches!(
        current(&authority, over_text.as_bytes()),
        Err(LiteralReadError::Preflight(
            PreflightFailure::StructuralLimit
        ))
    ));
    let ledger = DecodeLedger::new(&authority, 1024 * 1024).unwrap();
    let before = ledger.live_requested_bytes();
    let deep = format!("{}0{}", "[".repeat(131072), "]".repeat(131072));
    assert!(matches!(
        preflight(Cursor::new(deep), &ledger),
        Err(LiteralReadError::Preflight(
            PreflightFailure::StructuralLimit
        ))
    ));
    assert!(ledger.peak_requested_bytes() <= 1024 * 1024);
    assert_eq!(ledger.live_requested_bytes(), before);
}
#[test]
fn malformed_unicode_structure_and_numbers_remain_invalid_in_iterative_preflight() {
    let authority = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    for bytes in [
        br#"{"x":"\uD800"}"#.as_slice(),
        br#"{"x":01}"#,
        br#"{"x":1e999999}"#,
        br#"[1,]"#,
        br#"[}"#,
        br#"{}0"#,
    ] {
        assert!(former(bytes).is_err());
        assert!(current(&authority, bytes).is_err());
    }
}
