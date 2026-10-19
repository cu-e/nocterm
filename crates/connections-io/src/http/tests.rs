use super::*;
use crate::test_support::serve;

fn lookup(body: &[u8]) -> Result<Option<CountryCode>, ObservationError> {
    let (endpoint, worker) = serve(200, body.to_vec());
    let found =
        HttpCountry::at(endpoint).country(PublicAddress::new("8.8.8.8".parse().unwrap()).unwrap());
    let request = worker.join().unwrap();
    assert!(request.starts_with("GET /8.8.8.8 "));
    found
}

#[test]
fn country_requests_use_only_ip_and_decode_checked_optional_codes() {
    assert_eq!(
        lookup(br#"{"country":"DE"}"#).unwrap().unwrap().as_str(),
        "de"
    );
    assert_eq!(lookup(br#"{"country":null}"#).unwrap(), None);
    assert_eq!(lookup(br#"{}"#).unwrap(), None);
    for invalid in [&b"<html>"[..], &br#"{"country":"../"}"#[..]] {
        assert!(matches!(lookup(invalid), Err(ObservationError::Decode(_))));
    }
}

#[test]
fn status_error_retains_native_cause_without_endpoint() {
    use std::error::Error as _;
    let (endpoint, worker) = serve(503, vec![]);
    let result = get(&endpoint, MAX_GEOIP_BYTES).unwrap_err();
    worker.join().unwrap();
    assert!(matches!(
        result,
        ObservationError::Status { status: 503, .. }
    ));
    assert!(!result.source().unwrap().to_string().contains(&endpoint));
}

#[test]
fn country_response_limit_accepts_exact_limit_and_rejects_one_more() {
    let mut body = br#"{"country":"de"}"#.to_vec();
    body.resize(MAX_GEOIP_BYTES as usize, b' ');
    assert_eq!(lookup(&body).unwrap().unwrap().as_str(), "de");
    body.push(b' ');
    assert!(matches!(
        lookup(&body),
        Err(ObservationError::BodyLimit {
            limit: MAX_GEOIP_BYTES
        })
    ));
}
