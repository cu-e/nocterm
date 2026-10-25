//! The session store reads like the file store and keeps everything else in memory.

use std::fs;

use nocterm_plugin_api::{ContentHash, PackageError, PluginPackages as _};
use tempfile::TempDir;

use super::{PACKAGE, id, write};
use crate::{SessionStore, parse_package};

fn session() -> SessionStore {
    SessionStore::new(vec!["Files".into()])
}

#[test]
fn a_package_is_read_from_a_file_like_the_file_store_reads_it() {
    let directory = TempDir::new().unwrap();
    let location = write(directory.path(), "clock.toml", PACKAGE);
    let package = session().read(&location).unwrap();
    assert_eq!(package.bytes(), PACKAGE.as_bytes());
    assert_eq!(package.manifest().plugin().id(), &id());
    assert_eq!(
        session()
            .read(directory.path().join("nope.toml").to_str().unwrap())
            .unwrap_err(),
        PackageError::NotFound
    );
}

#[test]
fn reserved_names_are_refused_when_reading() {
    let directory = TempDir::new().unwrap();
    let location = write(
        directory.path(),
        "files.toml",
        &PACKAGE.replace("Acme Clock", "Files"),
    );
    assert!(session().read(&location).is_err());
}

#[test]
fn a_published_package_is_loaded_while_its_hash_holds_and_never_written() {
    let directory = TempDir::new().unwrap();
    let store = session();
    let package = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    store.publish(&package, false).unwrap();

    assert_eq!(store.load(&id(), package.hash()).unwrap(), package);
    assert_eq!(
        store.load(&id(), &ContentHash::new([0; 32])).unwrap_err(),
        PackageError::HashChanged
    );
    store.remove(&id()).unwrap();
    assert_eq!(
        store.load(&id(), package.hash()).unwrap_err(),
        PackageError::NotFound
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn publishing_the_same_plugin_twice_is_a_duplicate_unless_it_replaces() {
    let store = session();
    let first = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    let second = parse_package(format!("{PACKAGE}\n").into_bytes(), &[]).unwrap();
    store.publish(&first, false).unwrap();
    assert_eq!(
        store.publish(&second, false).unwrap_err(),
        PackageError::Duplicate(id())
    );
    store.publish(&second, true).unwrap();
    assert_eq!(store.load(&id(), second.hash()).unwrap(), second);
    assert_eq!(
        store.load(&id(), first.hash()).unwrap_err(),
        PackageError::HashChanged
    );
}

#[test]
fn removing_a_package_that_is_not_held_succeeds() {
    assert_eq!(session().remove(&id()), Ok(()));
}
