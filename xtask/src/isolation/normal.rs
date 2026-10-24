//! Test-only features in the normal closure. `cargo metadata` unifies the features that
//! dev-dependencies enable, so the question needs `cargo tree` with dev edges left out.
use std::{collections::BTreeSet, path::Path, process::Command};

use anyhow::bail;

use super::policy::Policy;

/// `cargo tree` of the app with `feature`, normal and build edges only, one `package|features`
/// line per package.
pub(super) fn tree(root: &Path, feature: &str) -> anyhow::Result<String> {
    let output = Command::new("cargo")
        .args(["tree", "-p", "nocterm", "--features", feature])
        .args(["-e", "normal,build", "--prefix", "none", "-f", "{p}|{f}"])
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
    let mut problems = BTreeSet::new();
    for line in tree.lines() {
        let Some((package, features)) = line.split_once('|') else {
            continue;
        };
        let Some(name) = package.split_whitespace().next() else {
            continue;
        };
        let Some(feature) = policy.compile_out.test_features.get(name) else {
            continue;
        };
        if features.split(',').any(|enabled| enabled.trim() == feature) {
            problems.insert(format!(
                "compile-out: {name} enables test-only feature {feature} in the normal build"
            ));
        }
    }
    problems
}
