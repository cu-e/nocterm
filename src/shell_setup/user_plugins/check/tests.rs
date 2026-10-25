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
    assert!(report.to_string().contains("no `.toml` package"));
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
