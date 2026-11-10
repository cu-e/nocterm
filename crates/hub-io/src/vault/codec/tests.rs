use std::path::PathBuf;

use nocterm_session::Target;

use super::*;

fn round_trip(binding: CredentialBinding) {
    let bytes = encode("label", &binding, &Secret::new("s3cret")).unwrap();
    match decode(&bytes) {
        Decoded::Known(record) => {
            assert_eq!(record.label, "label");
            assert_eq!(record.binding, binding);
            assert_eq!(record.secret.expose(), "s3cret");
        }
        Decoded::Unrecognized => panic!("a record this client wrote must read back"),
    }
}

#[test]
fn every_binding_kind_round_trips() {
    round_trip(CredentialBinding::password([Target::new(
        "ann",
        "example.org",
        22,
    )]));
    round_trip(CredentialBinding::KeyPassphrase {
        path: PathBuf::from("/home/ann/.ssh/id_ed25519"),
    });
    round_trip(CredentialBinding::PrivateKey {
        public_key: "ssh-ed25519 AAAAC3Nza".into(),
    });
}

#[test]
fn a_binding_of_an_unknown_kind_is_unrecognized() {
    let text = br#"{"v":1,"label":"x","binding":{"kind":"hardware_token","slot":3},"secret":"s"}"#;
    assert!(matches!(decode(text), Decoded::Unrecognized));
}

#[test]
fn a_newer_format_version_is_unrecognized() {
    let text = br#"{"v":2,"label":"x","binding":{"kind":"private_key","public_key":"k"},"secret":"s","extra":1}"#;
    assert!(matches!(decode(text), Decoded::Unrecognized));
}

#[test]
fn text_that_is_not_a_record_is_unrecognized() {
    assert!(matches!(decode(b"not json"), Decoded::Unrecognized));
    assert!(matches!(decode(br#"{"v":1}"#), Decoded::Unrecognized));
}

#[test]
fn a_password_the_web_panel_keeps_reads_here_bound_to_its_address() {
    // Exactly what the web panel seals (`sealPassword` of `nocterm-keys-wasm`): the port only
    // when it is not 22.
    for (target, port) in [
        (
            serde_json::json!({"host": "prod.example.com", "user": "ops"}),
            22,
        ),
        (
            serde_json::json!({"host": "prod.example.com", "port": 2222, "user": "ops"}),
            2222,
        ),
    ] {
        let plain = serde_json::json!({
            "v": 1,
            "label": "ops@prod.example.com",
            "binding": {"kind": "password", "target": target},
            "secret": "s3cret",
        });
        let Decoded::Known(record) = decode(plain.to_string().as_bytes()) else {
            panic!("the web panel's record reads here");
        };
        assert_eq!(
            record.binding,
            CredentialBinding::password([Target::new("ops", "prod.example.com", port)])
        );
        assert_eq!(record.secret.expose(), "s3cret");
    }
}

#[test]
fn a_key_the_web_panel_keeps_reads_here_bound_to_its_public_half() {
    // What `sealSshKey` of `nocterm-keys-wasm` seals: the key without its passphrase.
    let public_key =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAID7XjK7UMquFonNQf6mOb5ODK/CIOubLkjrBcI6wLh5+";
    let plain = serde_json::json!({
        "v": 1,
        "label": "prod",
        "binding": {"kind": "private_key", "public_key": public_key},
        "secret": "-----BEGIN OPENSSH PRIVATE KEY-----\n…\n-----END OPENSSH PRIVATE KEY-----\n",
    });
    let Decoded::Known(record) = decode(plain.to_string().as_bytes()) else {
        panic!("the web panel's key reads here");
    };
    assert_eq!(
        record.binding,
        CredentialBinding::PrivateKey {
            public_key: public_key.to_owned(),
        }
    );
    assert_eq!(record.label, "prod");
}
