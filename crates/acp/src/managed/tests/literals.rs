//! Surviving opaque JSON retains original insertion order and escaped key identity.
use super::{Fixture, fixture};
use crate::managed::{budget::SourceBudget, scan::Scanner, value::Value};
use agent_client_protocol::managed::Source;
use nocterm_ai::{
    history::source::SourceProducer,
    model::retention::{RetentionAuthority, document::DocumentRetention},
};

fn compare(raw: &str, fixture: &Fixture, observed_peak_bound: Option<usize>) {
    let source = Source::copy(raw, fixture.budget.clone()).unwrap();
    Scanner::new(raw, fixture.budget.clone())
        .complete()
        .unwrap();
    let node = Value::new(&source, 0..raw.len(), &fixture.budget)
        .literal()
        .unwrap();
    let decoded = node.into_decoded(fixture.budget.ledger()).unwrap();
    if let Some(bound) = observed_peak_bound {
        assert!(
            fixture.budget.ledger().peak_requested_bytes() <= bound,
            "actual Source64MiB ledger observed peak:{} > {}",
            fixture.budget.ledger().peak_requested_bytes(),
            bound
        );
    }
    let original = serde_json::from_str::<serde_json::Value>(raw).unwrap();
    assert_eq!(decoded.as_owned().as_value(), &original);
    assert_eq!(
        serde_json::to_string(decoded.as_owned().as_value()).unwrap(),
        serde_json::to_string(&original).unwrap()
    );
}
#[test]
fn opaque_objects_preserve_first_ordinal_and_last_winner_for_decoded_keys() {
    let fixture = fixture();
    for raw in [
        r#"{"a":{"discard":true},"b":2,"\u0061":3}"#,
        r#"{"😀":1,"x":[{"é":2,"\u00e9":3}],"\ud83d\ude00":4}"#,
        r#"{"a/b":1,"a\/b":2,"a\\b":3,"q\"":4}"#,
        r#"[{"x":1,"x":2},[],{},null,true,1,"text"]"#,
    ] {
        compare(raw, &fixture, None);
    }
}
#[test]
fn million_discarded_values_and_large_overwritten_text_keep_observed_source_peak_below_twenty_four_mib()
 {
    // SourceContext uses its original64MiB ledger. Authority::new checks only
    // construction metadata affordability; its argument is not a ledger limit.
    let authority = RetentionAuthority::new(24 * 1024 * 1024).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let fixture = Fixture {
        budget: SourceBudget::new(lease.context().unwrap()),
        _lease: lease,
        _producer: producer,
    };
    let mut raw = String::from(r#"{"large":""#);
    raw.extend(std::iter::repeat_n('x', 5 * 1024 * 1024));
    raw.push_str(r#"","million":["#);
    for i in 0..1_000_000 {
        if i > 0 {
            raw.push(',');
        }
        raw.push_str("null");
    }
    raw.push_str(r#"],"last":true,"\u006carge":1,"million":2}"#);
    compare(&raw, &fixture, Some(24 * 1024 * 1024));
}
