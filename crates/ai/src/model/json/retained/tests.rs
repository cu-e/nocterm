//! Public borrowed components bind actual root and ledger storage, never fabricated keys.
use super::RetainedAllocation;
use crate::model::{
    extensions::Extensions,
    json::{
        OwnedJson,
        deferred::{DecodeLedger, literal::LiteralBuilder},
    },
};
fn json(ledger: &DecodeLedger, value: &str) -> OwnedJson {
    let mut builder = LiteralBuilder::new(ledger).unwrap();
    let mut text = builder.text().unwrap();
    text.append(value).unwrap();
    builder.string(text).unwrap();
    builder.finish().unwrap().as_owned().clone()
}
/// Fixed test-only set, not a production census or a source-count bound.
fn unique_bytes<'a>(
    components: [Option<RetainedAllocation<'a>>; 4],
    seed: Option<RetainedAllocation<'a>>,
) -> usize {
    let mut seen = [None::<RetainedAllocation<'a>>; 4];
    let mut count = 0;
    let mut total = 0;
    for component in components.into_iter().flatten() {
        if seed
            .as_ref()
            .is_some_and(|seed| seed.same_allocation(&component))
            || seen[..count]
                .iter()
                .flatten()
                .any(|prior| prior.same_allocation(&component))
        {
            continue;
        }
        total += component.bytes().unwrap();
        seen[count] = Some(component);
        count += 1;
    }
    total
}
#[test]
fn shared_ledger_has_one_real_metadata_component_and_two_distinct_root_components() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, 16384).unwrap();
    let baseline = ledger.live_requested_bytes();
    let first = json(&ledger, "first");
    let second = json(&ledger, "second");
    let [a, a_ledger] = first.retained_allocations();
    let [b, b_ledger] = second.retained_allocations();
    assert!(!a.unwrap().same_allocation(&b.unwrap()));
    assert!(a_ledger.unwrap().same_allocation(&b_ledger.unwrap()));
    assert!(!a.unwrap().same_allocation(&a_ledger.unwrap()));
    let roots = first.retained_heap_bytes() + second.retained_heap_bytes();
    assert_eq!(
        unique_bytes([a, a_ledger, b, b_ledger], None),
        roots + baseline
    );
    assert_eq!(
        unique_bytes(
            [a, a_ledger, b, b_ledger],
            Some(ledger.allocation_component())
        ),
        roots
    );
    assert_eq!(first.retained_heap_bytes(), a.unwrap().bytes().unwrap());
}
#[test]
fn distinct_equal_limit_ledgers_never_share_metadata_identity() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let left = DecodeLedger::new(&authority, 16384).unwrap();
    let right = DecodeLedger::new(&authority, 16384).unwrap();
    let baseline = left.live_requested_bytes() + right.live_requested_bytes();
    let first = json(&left, "first");
    let second = json(&right, "second");
    let [a, a_ledger] = first.retained_allocations();
    let [b, b_ledger] = second.retained_allocations();
    assert!(!a_ledger.unwrap().same_allocation(&b_ledger.unwrap()));
    assert_eq!(
        unique_bytes([a, a_ledger, b, b_ledger], None),
        first.retained_heap_bytes() + second.retained_heap_bytes() + baseline
    );
}
#[test]
fn weak_receipt_still_attributes_ledger_after_explicit_handles_and_payload_disappear() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let (receipt, baseline) = {
        let ledger = DecodeLedger::new(&authority, 16384).unwrap();
        let baseline = ledger.live_requested_bytes();
        let value = json(&ledger, "retained original allocation");
        let receipt = value.receipt().clone();
        drop(value);
        (receipt, baseline)
    };
    let [root, ledger] = receipt.retained_allocations();
    assert_eq!(root.unwrap().bytes(), receipt.retained_identity_bytes());
    assert_eq!(ledger.unwrap().bytes(), Some(baseline));
    assert_eq!(
        unique_bytes([root, ledger, None, None], None),
        receipt.retained_identity_bytes().unwrap() + baseline
    );
}
#[test]
fn untracked_roots_have_real_root_identity_and_no_controlled_ledger_component() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let first = OwnedJson::from_untracked(&authority, serde_json::Value::Null).unwrap();
    let clone = first.clone();
    let second = OwnedJson::from_untracked(&authority, serde_json::Value::Null).unwrap();
    let [a, ledger] = first.retained_allocations();
    let [b, _] = clone.retained_allocations();
    let [replacement, _] = second.retained_allocations();
    assert!(ledger.is_none());
    assert!(a.unwrap().same_allocation(&b.unwrap()));
    assert!(!a.unwrap().same_allocation(&replacement.unwrap()));
    assert_eq!(a.unwrap().bytes(), Some(first.retained_heap_bytes()));
}
#[test]
fn filtered_extension_semantics_keep_the_same_retained_backing_component() {
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
    let original_id = original.identity();
    let filtered = original.clone().without_client_display();
    let filtered_id = filtered.identity();
    assert!(!original_id.matches(&filtered));
    assert!(filtered_id.matches(&filtered));
    let [a, a_ledger] = original_id.retained_allocations();
    let [b, b_ledger] = filtered_id.retained_allocations();
    assert!(a.unwrap().same_allocation(&b.unwrap()));
    assert!(a_ledger.is_none() && b_ledger.is_none());
    assert_eq!(
        unique_bytes([a, a_ledger, b, b_ledger], None),
        original.retained_heap_bytes().unwrap()
    );
}
