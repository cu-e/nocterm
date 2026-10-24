//! AST-based architectural debt inventory, with no command that raises the baseline.
use anyhow::{Context as _, bail};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path};
mod baseline;
mod bindings;
mod docs;
mod includes;
mod metrics;
mod source;
mod symbols;
pub(crate) use metrics::{io_path, walk_paths};
#[cfg(test)]
pub(crate) use source::imports;
pub(crate) use source::{Module, attributes, flatten, load as load_modules, test_only};
pub(crate) use symbols::Symbols;
/// Production structs may have at most this many fields.
pub(crate) const MAX_FIELDS: usize = 15;
/// A local type may be implemented in at most this many source files.
pub(crate) const MAX_IMPL_FILES: usize = 4;
const IO_DEPENDENCIES: &[&str] = &[
    "reqwest",
    "gpui-pre-reqwest",
    "tokio",
    "tar",
    "flate2",
    "portable-pty",
    "russh",
    "russh-sftp",
    "image",
    "rustix",
    "nix",
    "fs2",
    "tempfile",
];
#[cfg(test)]
mod tests;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    #[serde(default)]
    dto: BTreeMap<String, String>,
    /// Parsed and enforced by the isolation check.
    #[serde(default, rename = "isolation")]
    _isolation: serde::de::IgnoredAny,
}

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let policy: Policy = toml::from_str(&fs::read_to_string(
        root.join("xtask/architecture-policy.toml"),
    )?)?;
    let modules = source::load(root)?;
    let symbols = symbols::Symbols::new(&modules);
    for (name, reason) in &policy.dto {
        if reason.trim().is_empty() || !symbols.contains(name) {
            bail!("invalid DTO policy entry {name}");
        }
    }
    let mut counts = metrics::collect(&modules, &symbols, &policy.dto)?;
    count_complexity(root, &mut counts)?;
    count_dependencies(root, &mut counts)?;
    let baseline = baseline::Counts::read(&root.join("xtask/baseline.toml"))?;
    baseline.check(&counts)?;
    if let Ok(reference) = std::env::var("NOCTERM_ARCHITECTURE_BASE") {
        let status = std::process::Command::new("python3")
            .args([
                "scripts/check-architecture-history.py",
                "--base",
                &reference,
            ])
            .current_dir(root)
            .status()?;
        if !status.success() {
            bail!("architecture history check failed");
        }
    }
    docs::check(root, &symbols)?;
    Ok(())
}

fn count_complexity(root: &Path, counts: &mut baseline::Counts) -> anyhow::Result<()> {
    for package in crate::metadata(root)?.packages {
        let directory = package
            .manifest_path
            .parent()
            .context("package directory")?;
        let mut files = Vec::new();
        for name in ["src", "tests", "examples"] {
            let path = directory.join(name);
            if path.is_dir() {
                crate::source_files(&path, &mut files)?;
            }
        }
        let mut count = 0;
        for file in files {
            let (number, problems) = crate::lint_policy::inspect(&fs::read_to_string(&file)?)?;
            if !problems.is_empty() {
                bail!("invalid complexity expectations in {}", file.display());
            }
            count += number;
        }
        if count > 0 {
            counts.set("complexity_expects", &package.name, count);
        }
    }
    Ok(())
}

fn count_dependencies(root: &Path, counts: &mut baseline::Counts) -> anyhow::Result<()> {
    for package in crate::metadata(root)?.packages {
        if package
            .metadata
            .pointer("/nocterm/layer")
            .and_then(serde_json::Value::as_str)
            != Some("domain")
        {
            continue;
        }
        for dependency in &package.dependencies {
            if dependency.kind.as_deref() != Some("dev")
                && IO_DEPENDENCIES.contains(&dependency.name.as_str())
            {
                counts.set(
                    "domain_dependencies",
                    &format!("{}::{}", package.name, dependency.name),
                    1,
                );
            }
        }
    }
    Ok(())
}
