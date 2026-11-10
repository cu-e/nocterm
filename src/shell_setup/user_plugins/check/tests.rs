//! The author's check accepts what an install would accept and says why it refuses the rest.
use std::{fs, path::Path};

use tempfile::TempDir;

use super::{Report, check_directory};
use crate::shell_setup::plugin_catalog;

fn check(directory: &Path) -> Report {
    check_directory(
        directory,
        &plugin_catalog().expect("the application catalog is valid"),
    )
}

fn examples() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins")
}

#[test]
fn the_shipped_examples_pass_and_show_what_the_user_would_consent_to() {
    let report = check(&examples());
    let text = report.to_string();

    assert!(report.passed(), "{text}");
    assert!(text.contains("Example Clock") && text.contains("Example Notes"));
    assert!(text.contains("publisher example (unverified)"));
    assert!(text.contains("needs clock.now: See the current time"));
}

#[test]
fn a_broken_package_fails_with_its_reason_beside_a_good_one() {
    let directory = TempDir::new().unwrap();
    fs::copy(
        examples().join("clock.toml"),
        directory.path().join("ok.toml"),
    )
    .unwrap();
    fs::write(directory.path().join("bad.toml"), "api = \"0.1\"\n").unwrap();

    let report = check(directory.path());
    let text = report.to_string();

    assert!(!report.passed());
    assert!(text.contains("ok    ") && text.contains("FAIL  "), "{text}");
}

#[test]
fn a_directory_without_packages_does_not_pass() {
    let directory = TempDir::new().unwrap();
    fs::write(directory.path().join("readme.txt"), "not a package").unwrap();

    let report = check(directory.path());

    assert!(!report.passed());
    assert!(report.to_string().contains("no `.toml` or `.wasm` package"));
}

#[test]
fn two_packages_with_one_id_fail_the_second() {
    let directory = TempDir::new().unwrap();
    for name in ["a.toml", "b.toml"] {
        fs::copy(examples().join("clock.toml"), directory.path().join(name)).unwrap();
    }

    let report = check(directory.path());

    assert!(!report.passed());
    assert!(
        report
            .to_string()
            .contains("another package in this directory")
    );
}

#[test]
fn a_missing_directory_is_reported() {
    let directory = TempDir::new().unwrap();

    let report = check(&directory.path().join("missing"));

    assert!(!report.passed());
    assert!(report.to_string().starts_with("error:"));
}

#[cfg(unix)]
#[test]
fn a_link_is_not_followed() {
    let directory = TempDir::new().unwrap();
    std::os::unix::fs::symlink(
        examples().join("clock.toml"),
        directory.path().join("link.toml"),
    )
    .unwrap();

    assert!(!check(directory.path()).passed());
}

#[cfg(unix)]
#[test]
fn a_file_name_with_control_characters_cannot_move_the_cursor() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join("bad\u{1b}[2J.toml"),
        "api = \"0.1\"\n",
    )
    .unwrap();

    let text = check(directory.path()).to_string();

    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(text.contains("bad?[2J.toml"), "{text:?}");
}

/// The manifest of a code package that shows one widget and needs the clock.
const CODE_MANIFEST: &str = r#"
api = "0.1"
publisher = "example"
id = "pulse"
name = "Example Pulse"
description = "A widget written as code."
runtime = "wasm"
capabilities = ["clock.now"]

[reasons]
"clock.now" = "To show the time"

[[contribution]]
local = "face"
kind = "widget"
title = "Pulse"
icon = "puzzle"
zone = "right"
"#;

/// A module that carries `CODE_MANIFEST` and answers `body` to every call. The canned reply is
/// the postcard of a view with one text node `hi`.
fn code_package(body: &str) -> Vec<u8> {
    code_package_replying(r"\00\01\00\00\02\02\68\69\00\00\00", body)
}

/// Like [`code_package`] with another canned reply at the address `REPLY` points to.
fn code_package_replying(reply: &str, body: &str) -> Vec<u8> {
    let section: String = CODE_MANIFEST
        .bytes()
        .map(|byte| format!("\\{byte:02x}"))
        .collect();
    wat::parse_str(format!(
        r#"(module
            (@custom "nocterm.manifest" "{section}")
            (memory (export "memory") 2)
            (data (i32.const 1024) "{reply}")
            (func (export "nocterm_abi_version") (result i32) (i32.const 1))
            (func (export "nocterm_alloc") (param i32) (result i32) (i32.const 65536))
            (func (export "nocterm_call") (param i32 i32) (result i64) {body}))"#
    ))
    .expect("the test module is valid WAT")
}

/// `1024 << 32 | 11`: the canned reply in memory.
const REPLY: &str = "(i64.const 4398046511115)";

#[test]
fn a_code_package_is_instantiated_and_rendered_like_an_installed_one() {
    let directory = TempDir::new().unwrap();
    fs::write(directory.path().join("pulse.wasm"), code_package(REPLY)).unwrap();

    let report = check(directory.path());
    let text = report.to_string();

    assert!(report.passed(), "{text}");
    assert!(text.contains("Example Pulse"), "{text}");
    assert!(text.contains("runs code: a WebAssembly module"), "{text}");
    assert!(text.contains("needs clock.now: "), "{text}");
}

#[test]
fn a_code_package_that_traps_fails_with_the_contribution_that_did_not_render() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join("pulse.wasm"),
        code_package("unreachable"),
    )
    .unwrap();

    let report = check(directory.path());
    let text = report.to_string();

    assert!(!report.passed());
    assert!(text.contains("`face` does not render"), "{text}");
}

#[test]
fn a_code_package_whose_view_the_host_would_refuse_fails_the_check() {
    // The postcard of a view with one button that has no key, which no installed plugin may show.
    let keyless_button = r"\00\01\00\00\06\02\68\69\00\00\00";
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join("pulse.wasm"),
        code_package_replying(keyless_button, REPLY),
    )
    .unwrap();

    let report = check(directory.path());
    let text = report.to_string();

    assert!(!report.passed(), "{text}");
    assert!(
        text.contains("`face` draws a view the host refuses"),
        "{text}"
    );
    assert!(text.contains("needs a key"), "{text}");
}

#[test]
fn a_module_without_a_manifest_section_fails() {
    let directory = TempDir::new().unwrap();
    fs::write(
        directory.path().join("bare.wasm"),
        wat::parse_str("(module)").unwrap(),
    )
    .unwrap();

    let report = check(directory.path());

    assert!(!report.passed());
    assert!(report.to_string().contains("FAIL  "));
}

#[test]
fn code_and_declarative_packages_are_checked_together() {
    let directory = TempDir::new().unwrap();
    fs::copy(
        examples().join("clock.toml"),
        directory.path().join("clock.toml"),
    )
    .unwrap();
    fs::write(directory.path().join("pulse.wasm"), code_package(REPLY)).unwrap();

    let report = check(directory.path());

    assert!(report.passed(), "{report}");
}
