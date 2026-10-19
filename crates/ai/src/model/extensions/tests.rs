use super::*;
use serde_json::json;
#[test]
fn filtering_shared_metadata_preserves_buffers_identity_and_retained_backing() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let mut payload = String::with_capacity(1024 * 1024);
    payload.push_str("inert");
    let original = Extensions::checked(
        &authority,
        Map::from_iter([
            (DISPLAY.into(), Value::String(payload)),
            ("vendor".into(), json!(null)),
        ]),
    )
    .unwrap();
    let pointer = original.get(DISPLAY).unwrap().as_str().unwrap().as_ptr();
    let identity = original.identity();
    let retained = original.retained_heap_bytes().unwrap();
    let clone = original.clone();
    assert!(identity.matches(&clone));
    let filtered = clone.without_client_display();
    assert!(!identity.matches(&filtered));
    assert_eq!(
        original.get(DISPLAY).unwrap().as_str().unwrap().as_ptr(),
        pointer
    );
    assert_eq!(filtered.retained_heap_bytes(), Some(retained));
    assert_eq!(filtered.len(), 1);
    assert!(!filtered.contains_key(DISPLAY));
    assert_eq!(
        serde_json::to_value(&filtered).unwrap(),
        json!({"vendor":null})
    );
    let filtered_id = filtered.identity();
    assert!(filtered_id.matches(&filtered.clone().without_client_display()));
    let replaced: Extensions =
        crate::model::decode::Decode::decode(&authority, json!({"vendor":null})).unwrap();
    assert_eq!(filtered, replaced);
    assert!(!filtered_id.matches(&replaced));
}
#[test]
fn identity_expiry_retains_inline_allocation_and_cannot_match_replacement() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let original = Extensions::checked(&authority, Map::new()).unwrap();
    let identity = original.identity();
    drop(original);
    assert_eq!(
        identity.retained_heap_bytes(),
        crate::model::json::OwnedJson::identity_allocation_bytes()
    );
    assert!(!identity.matches(&Extensions::checked(&authority, Map::new()).unwrap()));
}
