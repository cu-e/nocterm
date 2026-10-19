use super::*;
use nocterm_ai::model::retention::RetentionAuthority;

fn original<'a>(mut cause: &'a (dyn std::error::Error + 'static)) -> &'a serde_json::Error {
    loop {
        if let Some(error) = cause.downcast_ref::<serde_json::Error>() {
            return error;
        }
        cause = cause.source().expect("same concrete stock serde cause");
    }
}
#[test]
fn typed_versions_preserve_original_stock_acceptance_and_error_positions() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let baseline = ledger.live_requested_bytes();
    let mut cases: Vec<Vec<u8>> = [
        "0",
        "2",
        "4294967295",
        "4294967296",
        "-1",
        "1.2",
        "1e2",
        "1e9999",
        "null",
        "true",
        "false",
        "[1]",
        "{}",
        "\"\"",
        "\"many\\u007f\\u200b\\n名\"",
        "\"bad\\uD800\"",
        "\"bad\\q\"",
        "\"unterminated",
        "n",
        "",
        "2 true",
    ]
    .into_iter()
    .map(|value| value.as_bytes().to_vec())
    .collect();
    cases.push(b"\"invalid \xff\"".to_vec());
    cases.push(format!("\"{}\"", "名字\\n".repeat(10000)).into_bytes());
    for bytes in cases {
        let stock = serde_json::from_reader::<_, u32>(bytes.as_slice());
        let managed = version(&mut bytes.as_slice(), bytes.len(), &ledger);
        match (stock, managed) {
            (Ok(expected), Ok(actual)) => assert_eq!(expected, actual),
            (Err(expected), Err(actual)) => {
                let actual = original(&actual);
                let input = String::from_utf8_lossy(&bytes);
                assert_eq!(actual.classify(), expected.classify(), "{input}");
                assert_eq!(actual.line(), expected.line(), "{input}");
                assert_eq!(actual.column(), expected.column(), "{input}");
                assert_eq!(actual.to_string(), expected.to_string(), "{input}");
            }
            (expected, actual) => panic!("stock={expected:?}, managed={actual:?}"),
        }
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
}
#[test]
fn null_marker_uses_original_unit_parser_and_releases_prospective_error_body() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let baseline = ledger.live_requested_bytes();
    for bytes in [b"null".as_slice(), b"n", b"nu", b"nul", b"nulx", b"null 1"] {
        let stock = serde_json::from_reader::<_, ()>(bytes);
        let managed = null(bytes, &ledger);
        match (stock, managed) {
            (Ok(()), Ok(())) => {}
            (Err(expected), Err(actual)) => {
                let actual = original(&actual);
                assert_eq!(actual.classify(), expected.classify());
                assert_eq!(actual.line(), expected.line());
                assert_eq!(actual.column(), expected.column());
                assert_eq!(actual.to_string(), expected.to_string());
            }
            (expected, actual) => panic!("stock={expected:?}, managed={actual:?}"),
        }
        assert_eq!(ledger.live_requested_bytes(), baseline);
    }
}
#[test]
fn numeric_spelling_length_does_not_admit_string_scratch_or_proportional_message() {
    let short = Profile::version(Some(b'1'), 1).unwrap();
    let long = Profile::version(Some(b'1'), 100_000_000).unwrap();
    assert_eq!(short.scratch, 0);
    assert_eq!(long.scratch, 0);
    assert_eq!(short.error, long.error);
    assert!(Profile::version(Some(b'"'), 100_000_000).unwrap().scratch > 100_000_000);
}

#[test]
fn route_stock_refusal_happens_before_string_parser_consumes_or_allocates() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let probe = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let baseline = probe.live_requested_bytes();
    drop(probe);
    let ledger = DecodeLedger::new(&authority, baseline).unwrap();
    let mut bytes = Cursor::new(b"\"original input\"".as_slice());
    assert!(matches!(
        version(&mut bytes, 16, &ledger),
        Err(RouteError::Admission(_))
    ));
    assert_eq!(bytes.position(), 1);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}

#[test]
fn escaped_typed_version_error_keeps_same_original_stock_cause_and_exact_credit() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let ledger = DecodeLedger::new(&authority, usize::MAX).unwrap();
    let baseline = ledger.live_requested_bytes();
    let result = version(&mut b"\"original value\"".as_slice(), 16, &ledger).unwrap_err();
    let RouteError::ManagedJson(error) = result else {
        panic!("original stock failure")
    };
    let cause = original(&error) as *const serde_json::Error;
    let charged = ledger.live_requested_bytes();
    let escaped = error.clone();
    assert_eq!(ledger.live_requested_bytes(), charged);
    drop(error);
    assert!(std::ptr::eq(original(&escaped), cause));
    assert_eq!(ledger.live_requested_bytes(), charged);
    drop(escaped);
    assert_eq!(ledger.live_requested_bytes(), baseline);
}
