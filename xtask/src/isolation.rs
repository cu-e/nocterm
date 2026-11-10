//! Guardrails that keep the plugin crates closed: the dependency graph, `std` reachability, the
//! gpui receivers, structural bans, provider crates and the features no build may hold
//! (docs/architecture/guardrails.md, ADR 0089).
//!
//! The rules guard against host-side regressions. They do not cover I/O reached through an
//! allowlisted dependency's own API, nor a confused deputy; those are design-review items.

use std::{collections::BTreeSet, path::Path};

use anyhow::bail;

use crate::ratchet;

mod gpui;
mod graph;
mod guest_only;
mod normal;
mod policy;
mod scan;
mod source;
#[cfg(test)]
mod tests;

use graph::Graph;
use policy::Policy;

pub(crate) fn check(
    root: &Path,
    modules: &[ratchet::Module],
    symbols: &ratchet::Symbols,
) -> anyhow::Result<()> {
    let policy = Policy::read(&root.join("xtask/architecture-policy.toml"))?;
    let default = Graph::read(root, &[])?;
    let gpui = if source::needs_gpui(&policy, modules) {
        gpui::receiver_methods(&default.package_directory(&policy.gpui)?.join("src"))?
    } else {
        BTreeSet::new()
    };
    let mut problems = graph::closure_problems(&policy, &default);
    problems.extend(graph::provider_problems(&policy, &default));
    problems.extend(guest_only::problems(&policy, &default, "default"));
    if !policy.features.test_features.is_empty() {
        problems.extend(normal::test_feature_problems(&policy, &normal::tree(root)?));
    }
    if !policy.features.banned_features.is_empty() {
        problems.extend(normal::banned_feature_problems(
            &policy,
            &normal::workspace_tree(root)?,
        ));
    }
    problems.extend(source::problems(&policy, modules, symbols, &gpui)?);
    if !problems.is_empty() {
        bail!(
            "plugin isolation violations:\n  {}",
            problems.into_iter().collect::<Vec<_>>().join("\n  ")
        );
    }
    Ok(())
}
