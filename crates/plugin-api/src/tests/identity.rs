use alloc::string::ToString;

use crate::{API_VERSION, ApiVersion, ContentHash, LocalId};

#[test]
fn a_local_name_is_lowercase_kebab_and_short() {
    assert!(LocalId::parse("face-2").is_ok());
    for bad in ["", "Face", "a.b", "a b", &"x".repeat(33)] {
        assert!(LocalId::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_hash_round_trips_through_lowercase_hex() {
    let hash = ContentHash::new([0xAB; 32]);
    let text = hash.to_string();
    assert_eq!(text.len(), 64);
    assert_eq!(ContentHash::parse(&text), Ok(hash));
    assert_eq!(hash.fingerprint(), "abababababababab");
    assert!(ContentHash::parse(&text.to_uppercase()).is_err());
    assert!(ContentHash::parse(&text[..62]).is_err());
}

#[test]
fn versions_parse_compare_and_gate_packages() {
    let version: ApiVersion = "0.1".parse().unwrap();
    assert_eq!(version, API_VERSION);
    assert!("0.2".parse::<ApiVersion>().unwrap() > version);
    for bad in ["1", "a.b", "1.", ".1", "1.2.3", "-1.0", "00000.1"] {
        assert!(bad.parse::<ApiVersion>().is_err(), "{bad:?}");
    }
    assert!(version.runs_on(ApiVersion::new(0, 1)));
    assert!(!ApiVersion::new(0, 2).runs_on(version));
    assert_eq!(version.to_string(), "0.1");
}
