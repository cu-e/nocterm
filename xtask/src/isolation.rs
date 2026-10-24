//! Guardrails that keep the plugin crates closed: the dependency graph, `std` reachability, the
//! gpui receivers, structural bans, provider crates and the compile-out of the user-plugin
//! track (docs/architecture/guardrails.md, ADR 0081).
//!
//! The rules guard against host-side regressions. They do not cover I/O reached through an
//! allowlisted dependency's own API, nor a confused deputy; those are design-review items.

use std::{collections::BTreeSet, path::Path};

use anyhow::bail;

use crate::ratchet;

mod gpui;
mod graph;
mod normal;
mod policy;
mod scan;
mod source;
#[cfg(test)]
mod tests;

use graph::Graph;
use policy::Policy;

/// Cargo feature that builds the user-plugin track; the real build, not `--all-features`.
const FEATURE: &str = "nocterm/user-plugins";

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let policy = Policy::read(&root.join("xtask/architecture-policy.toml"))?;
    let featured = Graph::read(root, &["--features", FEATURE])?;
    let default = Graph::read(root, &[])?;
    let modules = ratchet::load_modules(root)?;
    let symbols = ratchet::Symbols::new(&modules);
    let gpui = if source::needs_gpui(&policy, &modules) {
        gpui::receiver_methods(&featured.package_directory(&policy.gpui)?.join("src"))?
    } else {
        BTreeSet::new()
    };
    let mut problems = graph::closure_problems(&policy, &featured);
    problems.extend(graph::provider_problems(&policy, &featured));
    problems.extend(graph::compile_out_problems(&policy, &default));
    if !policy.compile_out.test_features.is_empty() {
        problems.extend(normal::test_feature_problems(
            &policy,
            &normal::tree(root, FEATURE)?,
        ));
    }
    problems.extend(source::problems(&policy, &modules, &symbols, &gpui)?);
    if !problems.is_empty() {
        bail!(
            "plugin isolation violations:\n  {}",
            problems.into_iter().collect::<Vec<_>>().join("\n  ")
        );
    }
    Ok(())
}
