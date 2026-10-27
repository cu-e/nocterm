use super::*;

const PRINT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn load(text: &str) -> HubsSettings {
    let table: toml::Table = text.parse().unwrap();
    let mut settings = table
        .get("hubs")
        .map(|hubs| hubs.clone().try_into::<HubsSettings>().unwrap())
        .unwrap_or_default();
    settings.sanitize();
    settings
}

fn pinned(address: &str, label: &str) -> HubRecord {
    HubRecord::new(
        HubAddress::parse(address).unwrap(),
        TrustPolicy::Pinned(Fingerprint::parse(PRINT).unwrap()),
        label,
    )
}

#[test]
fn servers_are_read_with_a_pinned_fingerprint_or_public_trust() {
    let settings = load(&format!(
        r#"
[[hubs.servers]]
address = "Hub.Home"
fingerprint = "{PRINT}"
label = "Home"

[[hubs.servers]]
address = "https://hub.example.com:8443/"
trust = "public-ca"
"#
    ));
    assert_eq!(settings.servers.len(), 2);
    assert_eq!(settings.servers[0], pinned("hub.home:7443", "Home"));
    assert_eq!(settings.servers[1].trust, TrustPolicy::PublicCa);
    assert_eq!(
        settings.servers[1].address.to_string(),
        "hub.example.com:8443"
    );
    assert_eq!(settings.servers[1].display_label(), "hub.example.com:8443");
}

#[test]
fn a_broken_server_does_not_drop_the_others() {
    let settings = load(&format!(
        r#"
[[hubs.servers]]
address = "http://plain.example.com"
trust = "public-ca"

[[hubs.servers]]
address = "no trust.example.com"

[[hubs.servers]]
address = "hub.home"
fingerprint = "not-hex"

[[hubs.servers]]
address = "hub.home"
trust = "everyone"

[[hubs.servers]]
address = "kept.home"
fingerprint = "{PRINT}"
"#
    ));
    assert_eq!(settings.servers, vec![pinned("kept.home", "")]);
}

#[test]
fn servers_with_the_same_address_collapse_to_the_first() {
    let settings = load(&format!(
        r#"
[[hubs.servers]]
address = "hub.home"
fingerprint = "{PRINT}"
label = "First"

[[hubs.servers]]
address = "HUB.home:7443"
trust = "public-ca"
label = "Second"
"#
    ));
    assert_eq!(settings.servers, vec![pinned("hub.home", "First")]);
}

#[test]
fn a_fingerprint_wins_over_public_trust_in_one_table() {
    let settings = load(&format!(
        r#"
[[hubs.servers]]
address = "hub.home"
fingerprint = "{PRINT}"
trust = "public-ca"
"#
    ));
    assert_eq!(settings.servers, vec![pinned("hub.home", "")]);
}

#[test]
fn saving_writes_only_the_known_fields() {
    let mut settings = HubsSettings::default();
    settings.upsert(pinned("hub.home", "Home"));
    settings.upsert(HubRecord::new(
        HubAddress::parse("hub.example.com").unwrap(),
        TrustPolicy::PublicCa,
        "",
    ));
    let text = toml::to_string(&toml::Table::from_iter([(
        "hubs".to_owned(),
        toml::Value::try_from(&settings).unwrap(),
    )]))
    .unwrap();
    assert!(text.contains("address = \"hub.home:7443\""));
    assert!(text.contains("trust = \"public-ca\""));
    assert!(!text.contains("label = \"\""), "{text}");
    assert!(!text.contains("token") && !text.contains("seed"), "{text}");
    assert_eq!(load(&text), settings);
}

#[test]
fn upsert_replaces_in_place_and_remove_forgets() {
    let mut settings = HubsSettings::default();
    settings.upsert(pinned("a.home", "A"));
    settings.upsert(pinned("b.home", "B"));
    settings.upsert(pinned("a.home", "Renamed"));
    assert_eq!(
        settings
            .servers
            .iter()
            .map(HubRecord::display_label)
            .collect::<Vec<_>>(),
        ["Renamed", "B"]
    );
    let id = HubId::from(HubAddress::parse("a.home").unwrap());
    assert!(settings.find(&id).is_some());
    assert!(settings.remove(&id));
    assert!(!settings.remove(&id));
    assert!(settings.find(&id).is_none());
}

#[test]
fn labels_lose_control_characters_and_are_shortened() {
    let long = "x".repeat(200);
    let record = pinned("hub.home", &format!(" Ho\nme\t{long}"));
    assert_eq!(record.label.chars().count(), 64);
    assert!(record.label.starts_with("Home"));
    assert!(!record.label.contains('\n'));
}

#[test]
fn a_missing_section_means_no_servers() {
    assert_eq!(load(""), HubsSettings::default());
    assert_eq!(HubsSettings::KEY, "hubs");
}
