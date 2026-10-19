//! First-party packages inherit the lint rules of their actual Cargo workspace.
use crate::Metadata;
use anyhow::{Context as _, bail};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
};

pub(crate) fn check(root: &Path, metadata: &Metadata) -> anyhow::Result<()> {
    validate_members(metadata).with_context(|| {
        format!(
            "{}: invalid workspace package discovery",
            root.join("Cargo.toml").display()
        )
    })?;
    let mut manifests: Vec<_> = metadata
        .packages
        .iter()
        .map(|package| &package.manifest_path)
        .collect();
    manifests.sort();
    for manifest in manifests {
        if !first_party(root, manifest)? {
            continue;
        }
        let source = fs::read_to_string(manifest).with_context(|| {
            format!(
                "{}: cannot read first-party Cargo manifest",
                manifest.display()
            )
        })?;
        inspect(manifest, &source)?;
    }
    Ok(())
}

fn validate_members(metadata: &Metadata) -> anyhow::Result<()> {
    if metadata.workspace_members.is_empty() {
        bail!("Cargo metadata has no workspace members");
    }
    let mut packages = BTreeSet::new();
    for package in &metadata.packages {
        if package.id.is_empty() || !packages.insert(package.id.as_str()) {
            bail!(
                "{}: empty or duplicate Cargo package ID {:?}",
                package.manifest_path.display(),
                package.id
            );
        }
    }
    let mut members = BTreeSet::new();
    for member in &metadata.workspace_members {
        if member.is_empty() || !members.insert(member.as_str()) {
            bail!("empty or duplicate Cargo workspace member ID {member:?}");
        }
        if !packages.contains(member.as_str()) {
            bail!("workspace member {member:?} has no discovered package");
        }
    }
    if let Some(package) = packages.difference(&members).next() {
        bail!("discovered Cargo package {package:?} is absent from workspace membership");
    }
    Ok(())
}

fn first_party(root: &Path, manifest: &Path) -> anyhow::Result<bool> {
    let relative = manifest.strip_prefix(root).with_context(|| {
        format!(
            "{}: workspace member manifest is outside {}",
            manifest.display(),
            root.display()
        )
    })?;
    let components: Vec<_> = relative.components().collect();
    if components
        .iter()
        .any(|component| !matches!(component, Component::Normal(_)))
        || relative.file_name() != Some(std::ffi::OsStr::new("Cargo.toml"))
    {
        bail!(
            "{}: unsupported workspace member manifest path",
            manifest.display()
        );
    }
    if relative == Path::new("Cargo.toml") || relative == Path::new("xtask/Cargo.toml") {
        return Ok(true);
    }
    if components.len() >= 3 {
        if components[0] == Component::Normal(std::ffi::OsStr::new("crates")) {
            return Ok(true);
        }
        if components[0] == Component::Normal(std::ffi::OsStr::new("vendor")) {
            return Ok(false);
        }
    }
    bail!(
        "{}: unsupported workspace member location",
        manifest.display()
    )
}

fn inspect(manifest: &Path, source: &str) -> anyhow::Result<()> {
    let document: toml::Value = toml::from_str(source)
        .with_context(|| format!("{}: cannot parse Cargo manifest", manifest.display()))?;
    if document
        .get("lints")
        .and_then(|lints| lints.get("workspace"))
        .and_then(toml::Value::as_bool)
        != Some(true)
    {
        bail!(
            "{}: first-party workspace package must inherit [lints] workspace = true",
            manifest.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
