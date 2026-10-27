use super::*;

fn parsed(text: &str) -> String {
    HubAddress::parse(text).unwrap().to_string()
}

#[test]
fn a_bare_host_gets_the_default_port_and_lower_case() {
    assert_eq!(parsed("Hub.Example.COM"), "hub.example.com:7443");
    assert_eq!(parsed("  hub.home  "), "hub.home:7443");
}

#[test]
fn an_explicit_port_is_kept() {
    let address = HubAddress::parse("hub.home:9000").unwrap();
    assert_eq!(address.host(), "hub.home");
    assert_eq!(address.port(), 9000);
    assert_eq!(parsed("10.0.0.5:7443"), "10.0.0.5:7443");
}

#[test]
fn ipv6_literals_need_brackets_and_keep_them_when_shown() {
    assert_eq!(parsed("[::1]:7443"), "[::1]:7443");
    assert_eq!(parsed("[::1]"), "[::1]:7443");
    assert_eq!(parsed("[FE80::1]:9"), "[fe80::1]:9");
    assert_eq!(
        HubAddress::parse("::1"),
        Err(HubAddressError::InvalidHost),
        "a second colon without brackets is ambiguous"
    );
    assert_eq!(HubAddress::parse("[::1"), Err(HubAddressError::InvalidHost));
    assert_eq!(
        HubAddress::parse("[not-an-address]:1"),
        Err(HubAddressError::InvalidHost)
    );
}

#[test]
fn https_is_dropped_and_http_is_refused() {
    assert_eq!(parsed("https://hub.example.com/"), "hub.example.com:7443");
    assert_eq!(
        parsed("HTTPS://hub.example.com:8443"),
        "hub.example.com:8443"
    );
    assert_eq!(
        HubAddress::parse("http://hub.example.com"),
        Err(HubAddressError::PlaintextRefused)
    );
    assert_eq!(
        HubAddress::parse("HTTP://127.0.0.1:7443"),
        Err(HubAddressError::PlaintextRefused)
    );
    assert_eq!(
        HubAddress::parse("ssh://hub.example.com"),
        Err(HubAddressError::UnsupportedScheme)
    );
}

#[test]
fn empty_and_garbage_addresses_are_refused() {
    assert_eq!(HubAddress::parse(""), Err(HubAddressError::Empty));
    assert_eq!(HubAddress::parse("   "), Err(HubAddressError::Empty));
    for garbage in [
        ":7443",
        "hub example",
        "hub.example.com/path",
        "user@hub.example.com",
        "-hub.example.com",
        "hub..example.com",
        "hub_example.com",
        "hub.example.com.",
    ] {
        assert_eq!(
            HubAddress::parse(garbage),
            Err(HubAddressError::InvalidHost),
            "{garbage}"
        );
    }
}

#[test]
fn ports_outside_the_range_are_refused() {
    for port in ["0", "65536", "-1", "abc", ""] {
        assert_eq!(
            HubAddress::parse(&format!("hub.home:{port}")),
            Err(HubAddressError::InvalidPort),
            "{port:?}"
        );
    }
    assert_eq!(parsed("hub.home:65535"), "hub.home:65535");
}

#[test]
fn spellings_of_one_hub_are_one_identity() {
    let first = HubId::from(HubAddress::parse("HUB.home").unwrap());
    let second = HubId::from(HubAddress::parse("https://hub.home:7443/").unwrap());
    assert_eq!(first, second);
    assert_eq!(first.to_string(), "hub.home:7443");
    assert_ne!(
        first,
        HubId::from(HubAddress::parse("hub.home:7444").unwrap())
    );
}

#[test]
fn an_address_round_trips_through_its_text() {
    let address = HubAddress::parse("[::1]:9").unwrap();
    assert_eq!(address.to_string().parse::<HubAddress>().unwrap(), address);
}
