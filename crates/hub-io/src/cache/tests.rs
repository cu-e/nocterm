use super::*;

fn address() -> HubAddress {
    "hub.example:7443".parse().unwrap()
}

fn record() -> v2::Keyset {
    v2::Keyset {
        version: 3,
        kdf: "argon2id;m=65536;t=3;p=4".into(),
        salt: vec![1; 16],
        wrapped_account_key: vec![2; 60],
        updated_at_ms: 9,
        recovery_wrapped_account_key: vec![5; 60],
        unlock_kind: v2::UnlockKind::LoginPassword as i32,
    }
}

fn account() -> AccountCache {
    let mut cache = AccountCache::new(
        "hub-uid".into(),
        "acc-1".into(),
        UnlockMethod::Login {
            username: "ann".into(),
        },
        &record(),
    );
    cache.vault = Some(Arc::new(VaultCopy {
        collection_id: "c1".into(),
        key_envelope: vec![7; 80],
        cursor: 2,
        objects: BTreeMap::from([(
            "o1".into(),
            ObjectCopy {
                version: 1,
                kind: "credential".into(),
                ciphertext: vec![9; 40],
                deleted: false,
            },
        )]),
    }));
    cache
}

#[test]
fn a_saved_account_reads_back_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let file = CacheFile::new(dir.path(), &address()).unwrap();
    assert!(file.load().unwrap().is_none());
    file.save(&account()).unwrap();
    assert_eq!(file.load().unwrap(), Some(account()));
    assert_eq!(account().keyset.record().wrapped_account_key, vec![2; 60]);
}

#[cfg(unix)]
#[test]
fn the_cache_is_private_and_holds_no_plain_text_of_the_secrets() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let file = CacheFile::new(dir.path(), &address()).unwrap();
    file.save(&account()).unwrap();
    let path = dir.path().join("hub.example_7443.cache");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!text.contains("password") && !text.contains("secret"));
}

#[test]
fn a_damaged_cache_is_reported_and_left_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let file = CacheFile::new(dir.path(), &address()).unwrap();
    let path = dir.path().join("hub.example_7443.cache");
    std::fs::write(&path, b"{ not json").unwrap();
    assert!(matches!(file.load(), Err(CacheError::Format(_))));
    assert!(path.exists());
}

#[test]
fn a_cache_from_a_newer_version_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let file = CacheFile::new(dir.path(), &address()).unwrap();
    std::fs::write(dir.path().join("hub.example_7443.cache"), br#"{"v":99}"#).unwrap();
    assert!(matches!(file.load(), Err(CacheError::Newer)));
}

#[test]
fn erasing_a_missing_cache_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = CacheFile::new(dir.path(), &address()).unwrap();
    file.erase().unwrap();
    file.save(&account()).unwrap();
    file.erase().unwrap();
    assert!(file.load().unwrap().is_none());
}

#[test]
fn an_oversized_cache_write_preserves_the_last_readable_account() {
    let dir = tempfile::tempdir().unwrap();
    let file = CacheFile::new(dir.path(), &address()).unwrap();
    file.save(&account()).unwrap();
    let mut oversized = account();
    oversized.hub_uid = "x".repeat(MAX_FILE_BYTES);
    assert!(matches!(file.save(&oversized), Err(CacheError::Limit)));
    assert_eq!(file.load().unwrap(), Some(account()));
}
