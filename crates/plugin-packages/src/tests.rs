//! Behavior of package reading, hashing, storing and removal.

use std::{fs, path::Path};

use nocterm_plugin_api::{
    API_VERSION, ContentHash, MAX_PACKAGE_BYTES, PackageError, PluginPackages as _,
};
use nocterm_plugin_model::PluginId;
use proptest::prelude::*;
use tempfile::TempDir;

use crate::{PackageStore, parse_package, sha256};

const PACKAGE: &str = r#"
api = "0.1"
publisher = "acme"
id = "clock"
name = "Acme Clock"
capabilities = ["clock.now"]

[[contribution]]
local = "face"
kind = "widget"
title = "Clock"

[contribution.view]
nodes = [{ kind = "text", text = "{clock.now|hh-mm}" }]
"#;

fn store(directory: &Path) -> PackageStore {
    PackageStore::new(directory.join("plugins"), vec!["Files".into()])
}

fn write(directory: &Path, name: &str, text: &str) -> String {
    let path = directory.join(name);
    fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_owned()
}

fn id() -> PluginId {
    PluginId::user("acme", "clock").unwrap()
}

#[test]
fn the_hash_is_a_standard_sha256_of_the_exact_bytes() {
    assert_eq!(
        sha256(b"abc").to_string(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn a_valid_package_is_read_with_the_hash_of_its_bytes() {
    let directory = TempDir::new().unwrap();
    let location = write(directory.path(), "clock.toml", PACKAGE);
    let package = store(directory.path()).read(&location).unwrap();
    assert_eq!(package.bytes(), PACKAGE.as_bytes());
    assert_eq!(*package.hash(), sha256(PACKAGE.as_bytes()));
    assert_eq!(package.manifest().plugin().id(), &id());
}

#[test]
fn a_different_byte_is_a_different_package() {
    let directory = TempDir::new().unwrap();
    let a = write(directory.path(), "a.toml", PACKAGE);
    let b = write(directory.path(), "b.toml", &format!("{PACKAGE}\n"));
    let store = store(directory.path());
    assert_ne!(
        store.read(&a).unwrap().hash(),
        store.read(&b).unwrap().hash()
    );
}

#[test]
fn unreadable_sources_are_refused_with_a_reason() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let missing = directory.path().join("none.toml");
    assert_eq!(
        store.read(missing.to_str().unwrap()).unwrap_err(),
        PackageError::NotFound
    );
    let folder = directory.path().to_str().unwrap();
    assert_eq!(store.read(folder).unwrap_err(), PackageError::NotRegular);
    let big = write(
        directory.path(),
        "big.toml",
        &"#".repeat(MAX_PACKAGE_BYTES as usize + 1),
    );
    assert_eq!(store.read(&big).unwrap_err(), PackageError::TooLarge);
    let padding = "#".repeat(MAX_PACKAGE_BYTES as usize - PACKAGE.len());
    let exact = write(
        directory.path(),
        "exact.toml",
        &format!("{PACKAGE}{padding}"),
    );
    assert!(store.read(&exact).is_ok());
}

#[cfg(unix)]
#[test]
fn links_and_pipes_are_not_followed_or_waited_on() {
    let directory = TempDir::new().unwrap();
    let real = write(directory.path(), "real.toml", PACKAGE);
    let link = directory.path().join("link.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let store = store(directory.path());
    assert_eq!(
        store.read(link.to_str().unwrap()).unwrap_err(),
        PackageError::NotRegular
    );
    let pipe = directory.path().join("pipe.toml");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(
        store.read(pipe.to_str().unwrap()).unwrap_err(),
        PackageError::NotRegular
    );
}

#[test]
fn text_that_is_not_a_valid_manifest_is_refused_before_consent() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let read = |name: &str, text: &str| store.read(&write(directory.path(), name, text));
    fs::write(directory.path().join("bytes.toml"), [0xFF, 0xFE]).unwrap();
    assert_eq!(
        store
            .read(directory.path().join("bytes.toml").to_str().unwrap())
            .unwrap_err(),
        PackageError::Encoding
    );
    assert!(matches!(
        read("a.toml", "api = [").unwrap_err(),
        PackageError::Syntax { .. }
    ));
    assert!(matches!(
        read(
            "b.toml",
            &PACKAGE.replace("name =", "website = \"x\"\nname =")
        )
        .unwrap_err(),
        PackageError::Syntax { .. }
    ));
    assert!(matches!(
        read("c.toml", &PACKAGE.replace("Acme Clock", "Files")).unwrap_err(),
        PackageError::Manifest(_)
    ));
}

#[test]
fn a_package_for_a_newer_api_says_so_even_with_keys_this_host_does_not_know() {
    let text = format!(
        "api = \"{}.{}\"\nfuture = 1\n",
        API_VERSION.major,
        API_VERSION.minor + 1
    );
    let error = parse_package(text.into_bytes(), &[]).unwrap_err();
    assert!(
        error.to_string().contains("Requires a newer Nocterm"),
        "{error}"
    );
}

#[test]
fn syntax_messages_do_not_echo_the_authors_text() {
    let secret = "VERY-LONG-AUTHOR-CONTROLLED-LINE-\u{202E}";
    let error = parse_package(format!("api = 1\n{secret} = = =").into_bytes(), &[]).unwrap_err();
    let PackageError::Syntax { reason } = error else {
        panic!("{error}")
    };
    assert!(
        !reason.contains("AUTHOR") && !reason.contains('\u{202E}'),
        "{reason}"
    );
}

#[test]
fn a_published_package_loads_only_with_the_consented_hash() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let package = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    store.publish(&package, false).unwrap();
    let loaded = store.load(&id(), package.hash()).unwrap();
    assert_eq!(loaded, package);
    assert_eq!(
        store.load(&id(), &ContentHash::new([0; 32])).unwrap_err(),
        PackageError::HashChanged
    );
}

#[cfg(unix)]
#[test]
fn published_files_are_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    store
        .publish(
            &parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap(),
            false,
        )
        .unwrap();
    let path = directory.path().join("plugins/user-acme-clock.toml");
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn tampering_after_install_is_detected_at_load() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let package = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    store.publish(&package, false).unwrap();
    let path = directory.path().join("plugins/user-acme-clock.toml");
    fs::write(&path, PACKAGE.replace("hh-mm", "percent")).unwrap();
    assert_eq!(
        store.load(&id(), package.hash()).unwrap_err(),
        PackageError::HashChanged
    );
}

#[cfg(unix)]
#[test]
fn a_stored_file_swapped_for_a_link_is_not_followed() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let package = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    store.publish(&package, false).unwrap();
    let path = directory.path().join("plugins/user-acme-clock.toml");
    let elsewhere = directory.path().join("elsewhere.toml");
    fs::write(&elsewhere, PACKAGE).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    assert_eq!(
        store.load(&id(), package.hash()).unwrap_err(),
        PackageError::NotRegular
    );
}

#[test]
fn a_second_install_is_a_duplicate_unless_it_is_an_update() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let first = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    store.publish(&first, false).unwrap();
    assert_eq!(
        store.publish(&first, false).unwrap_err(),
        PackageError::Duplicate(id())
    );
    let second = parse_package(format!("{PACKAGE}\n").into_bytes(), &[]).unwrap();
    store.publish(&second, true).unwrap();
    assert!(store.load(&id(), second.hash()).is_ok());
    assert_eq!(
        store.load(&id(), first.hash()).unwrap_err(),
        PackageError::HashChanged
    );
}

#[test]
fn a_package_whose_bytes_do_not_match_its_hash_is_not_published() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let real = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    let forged = nocterm_plugin_api::Package::new(
        ContentHash::new([9; 32]),
        real.manifest().clone(),
        real.bytes().to_vec(),
    );
    assert_eq!(
        store.publish(&forged, false).unwrap_err(),
        PackageError::HashChanged
    );
}

#[test]
fn removal_is_idempotent_and_files_are_named_only_by_user_ids() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let package = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    store.publish(&package, false).unwrap();
    store.remove(&id()).unwrap();
    store.remove(&id()).unwrap();
    assert_eq!(
        store.load(&id(), package.hash()).unwrap_err(),
        PackageError::NotFound
    );
    let system = PluginId::owned("monitor");
    assert_eq!(
        store.remove(&system),
        Err(PackageError::NotUserId(system.clone()))
    );
    let escape = PluginId::owned("user-../../x");
    assert!(store.load(&escape, package.hash()).is_err());
}

#[test]
fn a_failed_write_is_its_own_error_and_keeps_the_io_cause() {
    use std::error::Error as _;
    let directory = TempDir::new().unwrap();
    let blocker = directory.path().join("plugins");
    fs::write(&blocker, "a file where the directory should be").unwrap();
    let store = store(directory.path());
    let package = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    let error = store.publish(&package, false).unwrap_err();
    assert!(matches!(error, PackageError::Unwritable(_)), "{error:?}");
    let cause = error.source().expect("the I/O error is kept");
    assert!(!cause.to_string().is_empty());
}

#[test]
fn deeply_nested_tables_are_refused_not_recursed_into() {
    let depth = 200;
    let nested = format!("{}1{}", "{a=".repeat(depth), "}".repeat(depth));
    let text = format!("{PACKAGE}\n[contribution.view.extra]\nvalue = {nested}\n");
    let error = parse_package(text.into_bytes(), &[]).unwrap_err();
    let PackageError::Syntax { reason } = error else {
        panic!("{error:?}");
    };
    assert!(reason.contains("recursion"), "{reason}");
}

/// One edit of the bytes of a valid package: a byte replaced, a byte inserted or the tail cut.
fn damage(bytes: &mut Vec<u8>, at: usize, with: u8, how: u8) {
    if bytes.is_empty() {
        return;
    }
    let at = at % bytes.len();
    match how % 3 {
        0 => bytes[at] = with,
        1 => bytes.insert(at, with),
        _ => bytes.truncate(at),
    }
}

proptest! {
    #[test]
    fn arbitrary_bytes_are_refused_or_kept_with_the_hash_of_their_bytes(
        bytes in proptest::collection::vec(any::<u8>(), 0..400),
    ) {
        if let Ok(package) = parse_package(bytes.clone(), &[]) {
            prop_assert_eq!(package.bytes(), bytes.as_slice());
            prop_assert_eq!(*package.hash(), sha256(&bytes));
        }
    }

    #[test]
    fn a_damaged_package_never_panics_and_keeps_the_hash_of_what_was_read(
        edits in proptest::collection::vec((any::<usize>(), any::<u8>(), any::<u8>()), 1..6),
    ) {
        let mut bytes = PACKAGE.as_bytes().to_vec();
        for (at, with, how) in edits {
            damage(&mut bytes, at, with, how);
        }
        if let Ok(package) = parse_package(bytes.clone(), &["Files"]) {
            prop_assert_eq!(*package.hash(), sha256(&bytes));
            prop_assert!(package.manifest().plugin().id().is_user());
        }
    }
}
