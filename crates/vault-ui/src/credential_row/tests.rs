use super::*;
use nocterm_session::{CredentialId, Target};

const PUBLIC: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

#[test]
fn a_key_row_shows_the_fingerprint_other_rows_show_their_id() {
    let id = CredentialId::from_bytes([7; 16]);
    let key = nocterm_vault::CredentialInfo {
        id,
        label: "laptop".into(),
        binding: CredentialBinding::PrivateKey {
            public_key: PUBLIC.into(),
        },
    };
    let text = describe(&key);
    assert!(text.starts_with("SSH key"), "{text}");
    assert!(text.contains("SHA256:"), "{text}");
    let password = nocterm_vault::CredentialInfo {
        id,
        label: "host".into(),
        binding: CredentialBinding::Password {
            target: Target::new("test", "test-host", 22),
        },
    };
    assert_eq!(describe(&password), id.to_string());
}
