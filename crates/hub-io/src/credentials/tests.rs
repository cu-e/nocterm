use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::*;

fn address(text: &str) -> HubAddress {
    HubAddress::parse(text).unwrap()
}

fn session(token: &str) -> Session {
    Session {
        account_id: "account".into(),
        device_id: "device".into(),
        access_token: token.into(),
        access_expires_at_ms: 10,
        refresh_token: format!("refresh-{token}"),
        refresh_expires_at_ms: 20,
    }
}

fn file_in(dir: &tempfile::TempDir, text: &str) -> CredentialFile {
    CredentialDir::new(dir.path().join("hubs"))
        .file(&address(text))
        .unwrap()
}

#[test]
fn a_hub_without_a_file_has_no_device() {
    let dir = tempfile::tempdir().unwrap();
    assert!(file_in(&dir, "hub.example").load().unwrap().is_none());
}

#[test]
fn a_new_device_key_is_kept_and_found_again() {
    let dir = tempfile::tempdir().unwrap();
    let file = file_in(&dir, "hub.example");
    let (created, was_new) = file.load_or_create().unwrap();
    assert!(was_new);
    assert!(created.session.is_none());

    let (again, was_new) = file.load_or_create().unwrap();
    assert!(!was_new);
    assert_eq!(again.key.public_key(), created.key.public_key());
}

#[test]
fn tokens_are_saved_beside_the_key_and_cleared_without_losing_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = file_in(&dir, "hub.example");
    let (device, _) = file.load_or_create().unwrap();

    SessionStore::save(&file, &session("one")).unwrap();
    let stored = file.load().unwrap().unwrap();
    assert_eq!(stored.session, Some(session("one")));
    assert_eq!(stored.key.public_key(), device.key.public_key());
    assert_eq!(SessionStore::load(&file).unwrap(), Some(session("one")));

    file.clear_session().unwrap();
    let cleared = file.load().unwrap().unwrap();
    assert!(cleared.session.is_none());
    assert_eq!(cleared.key.public_key(), device.key.public_key());
}

#[test]
fn saving_tokens_for_a_hub_without_a_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    assert!(SessionStore::save(&file_in(&dir, "hub.example"), &session("one")).is_err());
}

#[cfg(unix)]
#[test]
fn the_file_and_its_directory_are_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let file = file_in(&dir, "hub.example");
    file.load_or_create().unwrap();
    SessionStore::save(&file, &session("one")).unwrap();

    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&file.path), 0o600);
    assert_eq!(mode(&file.lock_path), 0o600);
    assert_eq!(mode(file.path.parent().unwrap()), 0o700);
}

#[test]
fn erasing_removes_the_credentials_and_tolerates_their_absence() {
    let dir = tempfile::tempdir().unwrap();
    let file = file_in(&dir, "hub.example");
    file.load_or_create().unwrap();
    file.erase().unwrap();
    assert!(file.load().unwrap().is_none());
    file.erase().unwrap();
}

#[test]
fn every_hub_address_has_its_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let names: Vec<_> = ["a.example", "a.example:9", "b.example", "[::1]", "[::1]:9"]
        .iter()
        .map(|text| file_in(&dir, text).path)
        .collect();
    for (index, name) in names.iter().enumerate() {
        assert!(!names[..index].contains(name), "{name:?}");
    }
}

#[test]
fn a_file_of_another_hub_or_a_damaged_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let file = file_in(&dir, "hub.example");
    file.load_or_create().unwrap();
    let other = file_in(&dir, "other.example");
    fs::copy(&file.path, &other.path).unwrap();
    assert!(matches!(other.load(), Err(CredentialError::OtherHub)));

    fs::write(&file.path, "not toml [").unwrap();
    assert!(matches!(file.load(), Err(CredentialError::Format(_))));
    fs::write(
        &file.path,
        "address = \"hub.example:7443\"\nseed = \"abcd\"\n",
    )
    .unwrap();
    assert!(matches!(file.load(), Err(CredentialError::Seed(_))));
}

#[test]
fn an_address_too_long_for_a_file_name_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let label = "a".repeat(60);
    let host = format!("{label}.{label}.{label}.{label}.example");
    let result = CredentialDir::new(dir.path().to_owned()).file(&address(&host));
    assert!(matches!(result, Err(CredentialError::NameTooLong)));
}

#[test]
fn a_second_lock_waits_until_the_first_is_released() {
    let dir = tempfile::tempdir().unwrap();
    let first = file_in(&dir, "hub.example");
    let guard = SessionStore::lock(&first).unwrap();

    let second = file_in(&dir, "hub.example");
    let (acquired, signal) = mpsc::channel();
    let waiter = thread::spawn(move || {
        let _guard = SessionStore::lock(&second).unwrap();
        acquired.send(()).unwrap();
    });
    assert!(signal.recv_timeout(Duration::from_millis(200)).is_err());
    drop(guard);
    signal.recv_timeout(Duration::from_secs(10)).unwrap();
    waiter.join().unwrap();
}
