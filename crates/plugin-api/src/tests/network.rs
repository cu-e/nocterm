//! Requests to the user's web address: the `[network]` table, secret settings and request paths.

use alloc::{string::String, vec};

use super::support::{dto, host};
use crate::{
    Effect, EffectRequest, Effects, EffectsError, Manifest, ManifestError, NetworkError,
    capability::net::{Method, Request, check_path},
    capability::timers::SetTimer,
};

/// A code package that reaches the address of its `url` setting with the `token` setting.
const PANEL: &str = r#"
api = "0.1"
runtime = "wasm"
publisher = "acme"
id = "panel"
name = "Acme Panel"
capabilities = ["net.request"]

[[contribution]]
local = "servers"
kind = "panel"
title = "Servers"

[[settings.group]]
title = "Connection"

[[settings.group.field]]
key = "url"
label = "Panel address"
kind = "text"
max_len = 200

[[settings.group.field]]
key = "token"
label = "API key"
kind = "secret"
max_len = 128

[network]
origin_setting = "url"
credential_setting = "token"
"#;

fn validated(text: &str) -> Result<Manifest, ManifestError> {
    Manifest::validate(&dto(text), &host())
}

fn network_error(text: &str) -> NetworkError {
    match validated(text) {
        Err(ManifestError::Network(error)) => error,
        other => panic!("expected a network error, got {other:?}"),
    }
}

#[test]
fn a_package_names_the_settings_its_requests_use_and_the_header_defaults_to_a_bearer_token() {
    let manifest = validated(PANEL).expect("a valid package");
    let network = manifest.network().expect("a network table");
    assert_eq!(network.origin_setting(), "url");
    let credential = network.credential().expect("a credential");
    assert_eq!(credential.setting(), "token");
    assert_eq!(credential.header(), "Authorization");
    assert_eq!(credential.prefix(), "Bearer ");
    let schema = manifest.plugin().settings();
    assert!(schema.field("token").unwrap().kind.is_secret());
    assert!(!schema.field("url").unwrap().kind.is_secret());
}

#[test]
fn requests_need_the_table_and_the_table_needs_requests() {
    let without_table = PANEL
        .split("[network]")
        .next()
        .unwrap()
        .replace("kind = \"secret\"", "kind = \"text\"");
    assert_eq!(network_error(&without_table), NetworkError::Missing);
    let without_capability = PANEL.replace(r#"capabilities = ["net.request"]"#, "");
    assert_eq!(network_error(&without_capability), NetworkError::Unused);
}

#[test]
fn the_origin_is_a_plain_text_setting_and_the_credential_a_secret_one() {
    let origin_is_secret =
        PANEL.replace(r#"origin_setting = "url""#, r#"origin_setting = "token""#);
    assert_eq!(
        network_error(&origin_is_secret),
        NetworkError::Origin(String::from("token"))
    );
    let credential_is_plain = PANEL.replace(
        r#"credential_setting = "token""#,
        r#"credential_setting = "url""#,
    );
    assert_eq!(
        network_error(&credential_is_plain),
        NetworkError::Credential(String::from("url"))
    );
    let missing = PANEL.replace(r#"origin_setting = "url""#, r#"origin_setting = "nope""#);
    assert_eq!(
        network_error(&missing),
        NetworkError::Origin(String::from("nope"))
    );
}

#[test]
fn a_secret_the_requests_never_send_is_refused() {
    let unsent = PANEL.replace(r#"credential_setting = "token""#, "");
    assert_eq!(
        network_error(&unsent),
        NetworkError::StraySecret(String::from("token"))
    );
}

#[test]
fn the_credential_goes_into_authorization_or_an_x_header_only() {
    for (header, allowed) in [
        ("X-Api-Key", true),
        ("authorization", true),
        ("Cookie", false),
        ("Host", false),
        ("X-Bad Header", false),
    ] {
        let text = PANEL.replace(
            r#"credential_setting = "token""#,
            &alloc::format!("credential_setting = \"token\"\ncredential_header = \"{header}\""),
        );
        match validated(&text) {
            Ok(_) => assert!(allowed, "{header} should be refused"),
            Err(ManifestError::Network(NetworkError::Header)) => {
                assert!(!allowed, "{header} should be allowed")
            }
            Err(other) => panic!("{header}: {other:?}"),
        }
    }
}

#[test]
fn a_secret_setting_has_no_default_and_no_template_shows_it() {
    let defaulted = PANEL.replace("max_len = 128", "max_len = 128\ndefault = \"abc\"");
    assert!(validated(&defaulted).is_err());
    let shown = r#"
api = "0.1"
publisher = "acme"
id = "shown"
name = "Shown"

[[contribution]]
local = "face"
kind = "widget"
title = "Face"

[contribution.view]
nodes = [{ kind = "text", text = "{settings.token}" }]

[[settings.group]]
title = "G"

[[settings.group.field]]
key = "token"
label = "Key"
kind = "secret"
max_len = 10
"#;
    assert!(validated(shown).is_err(), "a template cannot read a secret");
}

#[test]
fn a_request_path_stays_below_the_origin() {
    for (path, fine) in [
        ("/api/items?page=1&per_page=25", true),
        ("/api/items/1a2b3c4d/files?directory=%2Fdocs", true),
        ("", false),
        ("api", false),
        ("//evil.example/steal", false),
        ("/a/../admin", false),
        ("/a/./b", false),
        ("/with space", false),
        ("/back\\slash", false),
        ("/frag#x", false),
        ("/caf\u{e9}", false),
    ] {
        assert_eq!(check_path(path).is_ok(), fine, "{path}");
    }
}

#[test]
fn a_request_and_a_timer_answer_under_a_key_they_must_carry() {
    let request = Effect::NetRequest(Request {
        method: Method::Get,
        path: String::from("/api"),
        body: None,
        bytes: false,
    });
    let timer = Effect::TimersSet(SetTimer { after_ms: 2000 });
    for effect in [request.clone(), timer] {
        let keyless = Effects(vec![EffectRequest { key: None, effect }]);
        assert_eq!(keyless.check(), Err(EffectsError::MissingKey { index: 0 }));
    }
    let keyed = Effects(vec![EffectRequest {
        key: Some(String::from("servers")),
        effect: request,
    }]);
    assert_eq!(keyed.check(), Ok(()));
    let too_soon = Effects(vec![EffectRequest {
        key: Some(String::from("t")),
        effect: Effect::TimersSet(SetTimer { after_ms: 10 }),
    }]);
    assert!(too_soon.check().is_err());
}

#[test]
fn an_origin_is_a_scheme_a_host_and_a_base_path_and_requests_stay_below_it() {
    use crate::capability::net::{Origin, OriginError};
    let origin = Origin::parse(" https://Service.Example.com:8443/base/ ").unwrap();
    assert!(origin.is_secure());
    assert_eq!(origin.authority(), "service.example.com:8443");
    assert_eq!(
        origin.url("/api/items").unwrap(),
        "https://service.example.com:8443/base/api/items"
    );
    assert!(origin.url("//evil.example").is_err());
    let lan = Origin::parse("http://192.168.1.20").unwrap();
    assert!(!lan.is_secure());
    assert_eq!(lan.url("/x?y=1").unwrap(), "http://192.168.1.20/x?y=1");
    assert!(Origin::parse("http://[::1]:8080").is_ok());
    for (text, error) in [
        ("", OriginError::Empty),
        ("ftp://host", OriginError::Scheme),
        ("service.example.com", OriginError::Scheme),
        ("https://user:pw@host", OriginError::Host),
        ("https://", OriginError::Host),
        ("https://host:99999", OriginError::Host),
        ("https://ho st", OriginError::Host),
        ("https://host?x=1", OriginError::Path),
        ("https://host/a/../b", OriginError::Path),
    ] {
        assert_eq!(Origin::parse(text), Err(error), "{text}");
    }
}
