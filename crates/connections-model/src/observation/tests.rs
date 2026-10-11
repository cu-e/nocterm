use super::*;

#[test]
fn country_keys_cannot_inject_paths_or_urls() {
    assert_eq!(CountryCode::new("de").unwrap().as_str(), "de");
    for invalid in ["", "d", "DE", "deu", "../", "d1", "ü", "a/", "a\\"] {
        assert!(CountryCode::new(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn country_serialization_preserves_strings_and_checks_input() {
    let code = CountryCode::new("de").unwrap();
    assert_eq!(serde_json::to_string(&code).unwrap(), "\"de\"");
    assert_eq!(serde_json::from_str::<CountryCode>("\"de\"").unwrap(), code);
    assert!(serde_json::from_str::<CountryCode>("\"../\"").is_err());
}

#[test]
fn public_address_policy_preserves_reserved_and_mapped_ranges() {
    for private in [
        "10.0.0.1",
        "172.16.4.2",
        "192.168.1.10",
        "127.0.0.1",
        "169.254.1.1",
        "100.64.0.1",
        "0.0.0.0",
        "255.255.255.255",
        "192.0.2.1",
        "198.18.0.1",
        "224.0.0.1",
        "::1",
        "fe80::1",
        "fd12:3456::1",
        "2001:db8::1",
        "::ffff:192.168.0.1",
        "240.0.0.1",
        "100.127.255.255",
    ] {
        assert!(
            PublicAddress::new(private.parse().unwrap()).is_none(),
            "{private}"
        );
    }
    for public in [
        "1.1.1.1",
        "8.8.8.8",
        "100.128.0.1",
        "2a00:1450:4001::1",
        "::ffff:8.8.8.8",
    ] {
        let ip = public.parse().unwrap();
        assert_eq!(PublicAddress::new(ip).unwrap().as_ip(), ip);
    }
}
