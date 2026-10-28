use super::*;
use gpui_kit::TestAppContext;
use nocterm_ssh_keys::KeyImportError;
use nocterm_vault::testing::ScriptedVaultAccess;

const ENCRYPTED: &str = include_str!("../../crates/ssh-keys/src/tests/fixtures/ed25519-encrypted");
const PLAIN: &str = include_str!("../../crates/ssh-keys/src/tests/fixtures/ed25519");
const PASSPHRASE: &str = "correct horse";

fn run<T>(cx: &TestAppContext, future: impl std::future::Future<Output = T>) -> T {
    cx.foreground_executor().block_test(future)
}

fn keys(cx: &TestAppContext) -> (VaultKeys, Arc<ScriptedVaultAccess>) {
    let vault = Arc::new(ScriptedVaultAccess::default());
    run(
        cx,
        vault.create(Secret::new("test unique master passphrase")),
    )
    .unwrap();
    (VaultKeys::new(vault.clone(), cx.executor().clone()), vault)
}

#[gpui_kit::test]
fn a_generated_key_is_listed_with_its_fingerprint_and_bound_to_its_public_half(
    cx: &mut TestAppContext,
) {
    let (keys, vault) = keys(cx);
    let stored = run(cx, keys.generate("laptop".into())).unwrap();
    assert!(stored.fingerprint.starts_with("SHA256:"));
    let listed = run(cx, keys.list()).unwrap();
    assert_eq!(listed, std::slice::from_ref(&stored));
    // The secret is the key itself and answers only for its own public half.
    let secret = run(
        cx,
        vault.get(
            stored.credential,
            CredentialBinding::PrivateKey {
                public_key: stored.public_key.as_str().to_owned(),
            },
        ),
    )
    .unwrap();
    let material = KeyMaterial::import(&secret, None).unwrap();
    assert_eq!(material.public_key(), stored.public_key);
    let other = CredentialBinding::PrivateKey {
        public_key:
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"
                .into(),
    };
    assert!(run(cx, vault.get(stored.credential, other)).is_err());
}

#[gpui_kit::test]
fn only_key_records_are_listed(cx: &mut TestAppContext) {
    let (keys, vault) = keys(cx);
    run(
        cx,
        vault.put(
            None,
            "host".into(),
            CredentialBinding::Password {
                target: nocterm_session::Target::new("me", "host", 22),
            },
            Secret::new("a password"),
        ),
    )
    .unwrap();
    assert!(run(cx, keys.list()).unwrap().is_empty());
    run(cx, keys.generate("laptop".into())).unwrap();
    assert_eq!(run(cx, keys.list()).unwrap().len(), 1);
}

#[gpui_kit::test]
fn an_encrypted_key_text_is_stored_without_its_passphrase(cx: &mut TestAppContext) {
    let (keys, vault) = keys(cx);
    let text = || KeySource::Text(Secret::new(ENCRYPTED));
    let missing = run(cx, keys.import("old".into(), text(), None)).unwrap_err();
    assert!(matches!(
        missing,
        VaultSshKeysError::Key(KeyImportError::PassphraseRequired)
    ));
    let wrong = run(
        cx,
        keys.import("old".into(), text(), Some(Secret::new("nope"))),
    )
    .unwrap_err();
    assert!(matches!(
        wrong,
        VaultSshKeysError::Key(KeyImportError::WrongPassphrase)
    ));
    assert!(
        run(cx, keys.list()).unwrap().is_empty(),
        "nothing stored on failure"
    );
    let stored = run(
        cx,
        keys.import("old".into(), text(), Some(Secret::new(PASSPHRASE))),
    )
    .unwrap();
    let secret = run(
        cx,
        vault.get(
            stored.credential,
            CredentialBinding::PrivateKey {
                public_key: stored.public_key.as_str().to_owned(),
            },
        ),
    )
    .unwrap();
    assert!(KeyMaterial::import(&secret, None).is_ok());
}

#[gpui_kit::test]
fn a_key_file_is_read_from_disk_and_a_missing_one_is_reported(cx: &mut TestAppContext) {
    let (keys, _vault) = keys(cx);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("id_ed25519");
    std::fs::write(&path, PLAIN).unwrap();
    let stored = run(cx, keys.import("file".into(), KeySource::File(path), None)).unwrap();
    assert_eq!(stored.label, "file");
    let missing = run(
        cx,
        keys.import(
            "file".into(),
            KeySource::File(directory.path().join("absent")),
            None,
        ),
    )
    .unwrap_err();
    assert!(matches!(missing, VaultSshKeysError::Read(_)), "{missing:?}");
}

#[gpui_kit::test]
fn a_locked_vault_serves_no_keys(cx: &mut TestAppContext) {
    let (keys, vault) = keys(cx);
    vault.lock();
    assert!(matches!(
        run(cx, keys.list()),
        Err(VaultSshKeysError::Locked)
    ));
    assert!(matches!(
        run(cx, keys.generate("k".into())),
        Err(VaultSshKeysError::Locked)
    ));
    assert!(matches!(
        run(
            cx,
            keys.import("k".into(), KeySource::Text(Secret::new(PLAIN)), None)
        ),
        Err(VaultSshKeysError::Locked)
    ));
}
