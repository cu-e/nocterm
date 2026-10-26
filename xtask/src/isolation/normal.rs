//! Features a resolved package must not hold. `cargo metadata` unifies the features that
//! dev-dependencies enable, so the question needs `cargo tree`: with dev edges left out for a
//! test-only feature, and over every member for a feature nothing may switch on.
use std::{collections::BTreeSet, path::Path, process::Command};

use anyhow::bail;

use super::policy::Policy;

/// `cargo tree` of the app with `feature`, normal and build edges only, one `package|features`
/// line per package.
pub(super) fn tree(root: &Path, feature: &str) -> anyhow::Result<String> {
    run(
        root,
        &["-p", "nocterm", "--features", feature, "-e", "normal,build"],
    )
}

/// `cargo tree` of every member with every feature and every kind of edge, in the same shape:
/// the features that any build of the workspace unifies.
pub(super) fn workspace_tree(root: &Path) -> anyhow::Result<String> {
    run(
        root,
        &["--workspace", "--all-features", "-e", "normal,build,dev"],
    )
}

fn run(root: &Path, selection: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("cargo")
        .arg("tree")
        .args(selection)
        .args(["--prefix", "none", "-f", "{p}|{f}"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        bail!(
            "cargo tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A package whose test-only feature a normal edge enables.
pub(super) fn test_feature_problems(policy: &Policy, tree: &str) -> BTreeSet<String> {
    enabled(&policy.compile_out.test_features, tree)
        .map(|(name, feature)| {
            format!("compile-out: {name} enables test-only feature {feature} in the normal build")
        })
        .collect()
}

/// A package that holds a feature the policy bans everywhere.
pub(super) fn banned_feature_problems(policy: &Policy, tree: &str) -> BTreeSet<String> {
    enabled(&policy.compile_out.banned_features, tree)
        .map(|(name, feature)| {
            format!("compile-out: {name} resolves with the banned feature {feature}")
        })
        .collect()
}

/// The `(package, feature)` pairs of `listed` that a `package|features` line of `tree` enables.
fn enabled<'a>(
    listed: &'a std::collections::BTreeMap<String, String>,
    tree: &'a str,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    tree.lines().filter_map(move |line| {
        let (package, features) = line.split_once('|')?;
        let name = package.split_whitespace().next()?;
        let feature = listed.get(name)?;
        features
            .split(',')
            .any(|enabled| enabled.trim() == feature)
            .then_some((name, feature.as_str()))
    })
}
