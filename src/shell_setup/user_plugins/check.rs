//! `nocterm plugin check <dir>`: reads every package of a directory the way an install does
//! and renders each contribution once, so an author sees what the user would be asked to
//! consent to and whether the views build, without installing anything.
use std::{
    fmt::{self, Write as _},
    path::{Path, PathBuf},
};

use nocterm_plugin_api::{
    ContributionContext, DeclarativeRuntime, Kind, LimitError, LocalId, Package, PackageError,
    PluginRuntime as _, RuntimeError, Snapshot, Zone,
};
use nocterm_plugin_model::{ContributionKind, PluginCatalog, PluginId};
use nocterm_plugin_packages::{CheckedFile, read_directory};

use super::reserved_names;

/// Why a package of the directory fails the check.
#[derive(Debug, thiserror::Error)]
enum CheckError {
    #[error("{0}")]
    Package(#[from] PackageError),
    #[error("another package in this directory is `{0}`")]
    Duplicate(PluginId),
    #[error("the runtime refuses it: {0}")]
    Runtime(#[from] RuntimeError),
    #[error("a contribution has no usable local name: {0}")]
    Local(#[from] LimitError),
    #[error("`{local}` does not render: {source}")]
    Render {
        local: LocalId,
        source: RuntimeError,
    },
}

/// What a directory held.
pub(crate) struct Report {
    files: Vec<(PathBuf, Result<String, CheckError>)>,
    problem: Option<String>,
}

impl Report {
    /// Whether every package passed and there was at least one.
    pub(crate) fn passed(&self) -> bool {
        self.problem.is_none()
            && !self.files.is_empty()
            && self.files.iter().all(|(_, outcome)| outcome.is_ok())
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(problem) = &self.problem {
            return writeln!(f, "error: {problem}");
        }
        if self.files.is_empty() {
            return writeln!(f, "error: no `.toml` package in this directory");
        }
        for (path, outcome) in &self.files {
            let name = shown(path);
            match outcome {
                Ok(summary) => writeln!(f, "ok    {name}\n{summary}")?,
                Err(reason) => writeln!(f, "FAIL  {name}\n      {reason}")?,
            }
        }
        Ok(())
    }
}

/// A path as the terminal may show it: a file name can hold control characters, which would
/// otherwise move the cursor or rewrite earlier lines of the report.
fn shown(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|character| {
            if character.is_control() {
                '?'
            } else {
                character
            }
        })
        .collect()
}

/// Checks the `.toml` files directly inside `directory`, in name order. Links and other
/// non-regular files are reported, not followed.
pub(crate) fn check_directory(directory: &Path, catalog: &PluginCatalog) -> Report {
    let reserved = reserved_names(catalog);
    let reserved: Vec<&str> = reserved.iter().map(String::as_str).collect();
    let files = match read_directory(directory, &reserved) {
        Ok(files) => files,
        Err(error) => {
            return Report {
                files: Vec::new(),
                problem: Some(error.to_string()),
            };
        }
    };
    let mut seen = Vec::new();
    let files = files
        .into_iter()
        .map(|CheckedFile { path, package }| {
            let outcome = package
                .map_err(CheckError::from)
                .and_then(|package| check_package(&package, &mut seen));
            (path, outcome)
        })
        .collect();
    Report {
        files,
        problem: None,
    }
}

fn check_package(package: &Package, seen: &mut Vec<PluginId>) -> Result<String, CheckError> {
    let id = package.manifest().plugin().id().clone();
    if seen.contains(&id) {
        return Err(CheckError::Duplicate(id));
    }
    seen.push(id);
    render_all(package)?;
    Ok(summary(package))
}

/// Renders each contribution against an empty snapshot: placeholders without an observation
/// show a dash, so only a view the host would refuse fails.
fn render_all(package: &Package) -> Result<(), CheckError> {
    let instance = DeclarativeRuntime.instantiate(package)?;
    for contribution in package.manifest().plugin().contributions() {
        let local = contribution
            .id()
            .as_str()
            .split_once('.')
            .map_or("", |(_, local)| local);
        let local = LocalId::parse(local)?;
        let (kind, zone) = match contribution.kind() {
            ContributionKind::Panel => (Kind::Panel, Zone::Left),
            ContributionKind::Widget => (Kind::Widget, Zone::Right),
        };
        let snapshot = Snapshot::new(ContributionContext { kind, zone });
        if let Err(source) = instance.render(&local, &snapshot) {
            return Err(CheckError::Render { local, source });
        }
    }
    Ok(())
}

/// What the consent dialog would show.
fn summary(package: &Package) -> String {
    let manifest = package.manifest();
    let plugin = manifest.plugin();
    let mut text = String::new();
    let _ = writeln!(
        text,
        "      {} ({}), publisher {} (unverified), fingerprint {}",
        plugin.name(),
        plugin.id(),
        manifest.publisher(),
        package.hash().fingerprint(),
    );
    for entry in manifest.required() {
        let _ = writeln!(text, "      needs {}: {}", entry.id, entry.consent);
    }
    for contribution in plugin.contributions() {
        let _ = writeln!(
            text,
            "      {:?} \"{}\"",
            contribution.kind(),
            contribution.title()
        );
    }
    text.trim_end().to_owned()
}

#[cfg(test)]
mod tests;
