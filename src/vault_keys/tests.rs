use super::*;
use futures::executor::block_on;
use nocterm_ssh_keys::KeyImportError;
use nocterm_vault::testing::ScriptedVaultAccess;

const ENCRYPTED: &str = include_str!("../../crates/ssh-keys/src/tests/fixtures/ed25519-encrypted");
const PLAIN: &str = include_str!("../../crates/ssh-keys/src/tests/fixtures/ed25519");
const PASSPHRASE: &str = "correct horse";

fn keys() -> (VaultKeys, Arc<ScriptedVaultAccess>) {
    let vault = Arc::new(ScriptedVaultAccess::default());
    block_on(vault.create(Secret::new("test unique master passphrase"))).unwrap();
    (VaultKeys::new(vault.clone()), vault)
}

#[test]
fn a_generated_key_is_listed_with_its_fingerprint_and_bound_to_its_public_half() {
    let (keys, vault) = keys();
    let stored = block_on(keys.generate("laptop".into())).unwrap();
    assert!(stored.fingerprint.starts_with("SHA256:"));
    let listed = block_on(keys.list()).unwrap();
    assert_eq!(listed, [stored.clone()]);
    // The secret is the key itself and answers only for its own public half.
    let secret = block_on(vault.get(
        stored.credential,
        CredentialBinding::PrivateKey {
            public_key: stored.public_key.as_str().to_owned(),
        },
    ))
    .unwrap();
    let material = KeyMaterial::import(&secret, None).unwrap();
    assert_eq!(material.public_key(), stored.public_key);
    let other = CredentialBinding::PrivateKey {
        public_key:
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"
                .into(),
    };
    assert!(block_on(vault.get(stored.credential, other)).is_err());
}

#[test]
fn only_key_records_are_listed() {
    let (keys, vault) = keys();
    block_on(vault.put(
        None,
        "host".into(),
        CredentialBinding::Password {
            target: nocterm_session::Target::new("me", "host", 22),
        },
        Secret::new("a password"),
    ))
    .unwrap();
    assert!(block_on(keys.list()).unwrap().is_empty());
    block_on(keys.generate("laptop".into())).unwrap();
    assert_eq!(block_on(keys.list()).unwrap().len(), 1);
}

#[test]
fn an_encrypted_key_text_is_stored_without_its_passphrase() {
    let (keys, vault) = keys();
    let text = || KeySource::Text(Secret::new(ENCRYPTED));
    let missing = block_on(keys.import("old".into(), text(), None)).unwrap_err();
    assert!(matches!(
        missing,
        VaultSshKeysError::Key(KeyImportError::PassphraseRequired)
    ));
    let wrong = block_on(keys.import("old".into(), text(), Some(Secret::new("nope")))).unwrap_err();
    assert!(matches!(
        wrong,
        VaultSshKeysError::Key(KeyImportError::WrongPassphrase)
    ));
    assert!(
        block_on(keys.list()).unwrap().is_empty(),
        "nothing stored on failure"
    );
    let stored =
        block_on(keys.import("old".into(), text(), Some(Secret::new(PASSPHRASE)))).unwrap();
    let secret = block_on(vault.get(
        stored.credential,
        CredentialBinding::PrivateKey {
            public_key: stored.public_key.as_str().to_owned(),
        },
    ))
    .unwrap();
    assert!(KeyMaterial::import(&secret, None).is_ok());
}

#[test]
fn a_key_file_is_read_from_disk_and_a_missing_one_is_reported() {
    let (keys, _vault) = keys();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("id_ed25519");
    std::fs::write(&path, PLAIN).unwrap();
    let stored = block_on(keys.import("file".into(), KeySource::File(path), None)).unwrap();
    assert_eq!(stored.label, "file");
    let missing = block_on(keys.import(
        "file".into(),
        KeySource::File(directory.path().join("absent")),
        None,
    ))
    .unwrap_err();
    assert!(matches!(missing, VaultSshKeysError::Read(_)), "{missing:?}");
}

#[test]
fn a_locked_vault_serves_no_keys() {
    let (keys, vault) = keys();
    vault.lock();
    assert!(matches!(
        block_on(keys.list()),
        Err(VaultSshKeysError::Locked)
    ));
    assert!(matches!(
        block_on(keys.generate("k".into())),
        Err(VaultSshKeysError::Locked)
    ));
    assert!(matches!(
        block_on(keys.import("k".into(), KeySource::Text(Secret::new(PLAIN)), None)),
        Err(VaultSshKeysError::Locked)
    ));
}
