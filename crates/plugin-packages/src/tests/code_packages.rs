//! Code packages: a WebAssembly module whose custom section holds the manifest. Packages are
//! never run here, so the modules are only a header and sections.

use std::fs;

use nocterm_plugin_api::{
    ContentHash, MANIFEST_SECTION, MAX_MODULE_BYTES, MAX_PACKAGE_BYTES, PackageError, PackageKind,
    PluginPackages as _, Runtime, SectionError,
};
use tempfile::TempDir;

use super::{PACKAGE, id, store, write};
use crate::{PackageStore, parse_package, read_directory, read_package, sha256};

const CODE_MANIFEST: &str = r#"
api = "0.1"
runtime = "wasm"
publisher = "acme"
id = "clock"
name = "Acme Clock"
capabilities = ["clock.now"]

[[contribution]]
local = "face"
kind = "widget"
title = "Clock"
"#;

fn leb(mut value: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

fn custom_section(name: &str, data: &[u8]) -> Vec<u8> {
    let mut payload = leb(name.len());
    payload.extend(name.as_bytes());
    payload.extend(data);
    let mut section = vec![0];
    section.extend(leb(payload.len()));
    section.extend(payload);
    section
}

const HEADER: &[u8] = b"\0asm\x01\0\0\0";

fn module(manifest: &str) -> Vec<u8> {
    let mut bytes = HEADER.to_vec();
    bytes.extend(custom_section(MANIFEST_SECTION, manifest.as_bytes()));
    bytes
}

fn write_bytes(directory: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = directory.join(name);
    fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn a_code_package_is_read_with_the_hash_of_the_whole_file() {
    let bytes = module(CODE_MANIFEST);
    let package = parse_package(bytes.clone(), &[]).unwrap();
    assert_eq!(package.manifest().runtime(), Runtime::Wasm);
    assert_eq!(package.manifest().plugin().id(), &id());
    assert_eq!(package.bytes(), bytes.as_slice());
    assert_eq!(*package.hash(), sha256(&bytes));
    assert!(
        package
            .manifest()
            .has_contribution(&nocterm_plugin_api::LocalId::parse("face").unwrap())
    );
}

#[test]
fn the_declared_runtime_must_match_the_kind_of_file() {
    assert_eq!(
        parse_package(CODE_MANIFEST.as_bytes().to_vec(), &[]).unwrap_err(),
        PackageError::RuntimeMismatch {
            declared: Runtime::Wasm,
            kind: PackageKind::Manifest,
        }
    );
    assert_eq!(
        parse_package(module(PACKAGE), &[]).unwrap_err(),
        PackageError::RuntimeMismatch {
            declared: Runtime::Declarative,
            kind: PackageKind::Module,
        }
    );
}

#[test]
fn a_module_without_a_readable_manifest_section_is_refused() {
    assert_eq!(
        parse_package(HEADER.to_vec(), &[]).unwrap_err(),
        PackageError::Section(SectionError::Missing)
    );
    assert_eq!(
        parse_package(b"\0asm\x02\0\0\0".to_vec(), &[]).unwrap_err(),
        PackageError::Section(SectionError::Header)
    );
    let mut truncated = module(CODE_MANIFEST);
    truncated.truncate(truncated.len() - 5);
    assert_eq!(
        parse_package(truncated, &[]).unwrap_err(),
        PackageError::Section(SectionError::Truncated)
    );
}

#[test]
fn a_manifest_that_is_not_text_or_not_valid_is_refused_inside_a_module() {
    let mut not_text = HEADER.to_vec();
    not_text.extend(custom_section(MANIFEST_SECTION, &[0xff, 0xfe]));
    assert_eq!(
        parse_package(not_text, &[]).unwrap_err(),
        PackageError::Encoding
    );
    assert!(matches!(
        parse_package(module("api = \"0.1\"\nruntime = \"wasm\"\n"), &[]).unwrap_err(),
        PackageError::Syntax { .. } | PackageError::Manifest(_)
    ));
}

#[test]
fn a_module_may_be_larger_than_a_manifest_but_not_than_its_own_limit() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let base = module(CODE_MANIFEST);
    // The padding is a second custom section: id, 3 size bytes, name length, "pad", data.
    let pad_to = |size: u64| {
        let mut bytes = base.clone();
        let room = size as usize - bytes.len() - 8;
        bytes.extend(custom_section("pad", &vec![0; room]));
        bytes
    };
    let exact = pad_to(MAX_MODULE_BYTES);
    assert_eq!(exact.len() as u64, MAX_MODULE_BYTES);
    let location = write_bytes(directory.path(), "exact.wasm", &exact);
    assert!(store.read(&location).is_ok());

    let mut over = exact;
    over.push(0);
    let location = write_bytes(directory.path(), "over.wasm", &over);
    assert_eq!(store.read(&location).unwrap_err(), PackageError::TooLarge);
}

#[test]
fn the_manifest_inside_a_module_has_the_limit_of_a_manifest() {
    let padded = format!("{CODE_MANIFEST}{}", "#".repeat(MAX_PACKAGE_BYTES as usize));
    assert_eq!(
        parse_package(module(&padded), &[]).unwrap_err(),
        PackageError::TooLarge
    );
}

#[test]
fn a_code_package_is_stored_loaded_and_checked_like_a_manifest() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let package = parse_package(module(CODE_MANIFEST), &[]).unwrap();
    store.publish(&package, false).unwrap();

    let path = directory.path().join("plugins/user-acme-clock.wasm");
    assert!(path.exists());
    assert!(
        !directory
            .path()
            .join("plugins/user-acme-clock.toml")
            .exists()
    );
    assert_eq!(store.load(&id(), package.hash()).unwrap(), package);
    assert_eq!(
        store.load(&id(), &ContentHash::new([0; 32])).unwrap_err(),
        PackageError::HashChanged
    );

    let mut tampered = fs::read(&path).unwrap();
    *tampered.last_mut().unwrap() ^= 1;
    fs::write(&path, tampered).unwrap();
    assert_eq!(
        store.load(&id(), package.hash()).unwrap_err(),
        PackageError::HashChanged
    );
}

#[cfg(unix)]
#[test]
fn a_stored_module_is_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    store
        .publish(&parse_package(module(CODE_MANIFEST), &[]).unwrap(), false)
        .unwrap();
    let path = directory.path().join("plugins/user-acme-clock.wasm");
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn an_update_that_changes_the_kind_leaves_one_file() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let declarative = parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap();
    let code = parse_package(module(CODE_MANIFEST), &[]).unwrap();
    let plugins = directory.path().join("plugins");

    store.publish(&declarative, false).unwrap();
    store.publish(&code, true).unwrap();
    assert!(plugins.join("user-acme-clock.wasm").exists());
    assert!(!plugins.join("user-acme-clock.toml").exists());
    assert_eq!(
        store.load(&id(), declarative.hash()).unwrap_err(),
        PackageError::HashChanged,
        "the replaced package is gone; only the new bytes are installed"
    );

    store.publish(&declarative, true).unwrap();
    assert!(plugins.join("user-acme-clock.toml").exists());
    assert!(!plugins.join("user-acme-clock.wasm").exists());
    assert_eq!(store.load(&id(), declarative.hash()).unwrap(), declarative);
}

#[test]
fn a_package_of_the_other_kind_makes_a_new_install_a_duplicate() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    store
        .publish(
            &parse_package(PACKAGE.as_bytes().to_vec(), &[]).unwrap(),
            false,
        )
        .unwrap();
    let code = parse_package(module(CODE_MANIFEST), &[]).unwrap();
    assert_eq!(
        store.publish(&code, false).unwrap_err(),
        PackageError::Duplicate(id())
    );
}

#[test]
fn removal_takes_away_both_kinds() {
    let directory = TempDir::new().unwrap();
    let store = store(directory.path());
    let plugins = directory.path().join("plugins");
    store
        .publish(&parse_package(module(CODE_MANIFEST), &[]).unwrap(), false)
        .unwrap();
    // A leftover of the other kind, as an interrupted update could leave.
    fs::write(plugins.join("user-acme-clock.toml"), PACKAGE).unwrap();

    store.remove(&id()).unwrap();

    assert_eq!(fs::read_dir(&plugins).unwrap().count(), 0);
    store.remove(&id()).unwrap();
}

#[test]
fn a_directory_check_lists_wasm_packages_with_toml_ones() {
    let directory = TempDir::new().unwrap();
    write(directory.path(), "a.toml", PACKAGE);
    write_bytes(directory.path(), "b.wasm", &module(CODE_MANIFEST));
    write(directory.path(), "c.txt", "not a package");

    let files = read_directory(directory.path(), &["Files"]).unwrap();

    let names: Vec<_> = files
        .iter()
        .map(|file| file.path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["a.toml", "b.wasm"]);
    assert_eq!(
        files[1].package.as_ref().unwrap().manifest().runtime(),
        Runtime::Wasm
    );
}

#[test]
fn read_package_checks_a_file_the_way_an_install_does() {
    let directory = TempDir::new().unwrap();
    let location = write_bytes(directory.path(), "clock.wasm", &module(CODE_MANIFEST));

    let package = read_package(std::path::Path::new(&location), &["Files"]).unwrap();

    assert_eq!(package.manifest().runtime(), Runtime::Wasm);
    assert_eq!(
        PackageStore::read_bytes(&directory.path().join("missing.wasm")).unwrap_err(),
        PackageError::NotFound
    );
}
