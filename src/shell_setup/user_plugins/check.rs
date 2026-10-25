//! `nocterm plugin check <dir>`: reads every package of a directory the way an install does
//! and renders each contribution once, so an author sees what the user would be asked to
//! consent to and whether the views build, without installing anything.
use std::{
    fmt::{self, Write as _},
    fs,
    path::{Path, PathBuf},
};

use nocterm_plugin_api::{
    ContributionContext, DeclarativeRuntime, Kind, LocalId, Package, PluginPackages as _,
    PluginRuntime as _, Snapshot, Zone,
};
use nocterm_plugin_model::{ContributionKind, PluginCatalog};
use nocterm_plugin_packages::PackageStore;

use super::reserved_names;

/// What a directory held.
pub(crate) struct Report {
    files: Vec<(PathBuf, Result<String, String>)>,
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
            match outcome {
                Ok(summary) => writeln!(f, "ok    {}\n{summary}", path.display())?,
                Err(reason) => writeln!(f, "FAIL  {}\n      {reason}", path.display())?,
            }
        }
        Ok(())
    }
}

/// Checks the `.toml` files directly inside `directory`, in name order. Links and other
/// non-regular files are reported, not followed.
pub(crate) fn check_directory(directory: &Path, catalog: &PluginCatalog) -> Report {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            return Report {
                files: Vec::new(),
                problem: Some(format!("{}: {error}", directory.display())),
            };
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    paths.sort();
    let store = PackageStore::new(directory.join(".none"), reserved_names(catalog));
    let mut seen = Vec::new();
    let files = paths
        .into_iter()
        .map(|path| {
            let outcome = check_file(&store, &path, &mut seen);
            (path, outcome)
        })
        .collect();
    Report {
        files,
        problem: None,
    }
}

fn check_file(
    store: &PackageStore,
    path: &Path,
    seen: &mut Vec<nocterm_plugin_model::PluginId>,
) -> Result<String, String> {
    let location = path.to_str().ok_or("the path is not text")?;
    let package = store.read(location).map_err(|error| error.to_string())?;
    let id = package.manifest().plugin().id().clone();
    if seen.contains(&id) {
        return Err(format!("another package in this directory is `{id}`"));
    }
    seen.push(id);
    render_all(&package)?;
    Ok(summary(&package))
}

/// Renders each contribution against an empty snapshot: placeholders without an observation
/// show a dash, so only a view the host would refuse fails.
fn render_all(package: &Package) -> Result<(), String> {
    let instance = DeclarativeRuntime
        .instantiate(package)
        .map_err(|error| error.to_string())?;
    for contribution in package.manifest().plugin().contributions() {
        let local = contribution
            .id()
            .as_str()
            .split_once('.')
            .map_or("", |(_, local)| local);
        let local = LocalId::parse(local).map_err(|error| error.to_string())?;
        let (kind, zone) = match contribution.kind() {
            ContributionKind::Panel => (Kind::Panel, Zone::Left),
            ContributionKind::Widget => (Kind::Widget, Zone::Right),
        };
        let snapshot = Snapshot::new(ContributionContext { kind, zone });
        instance
            .render(&local, &snapshot)
            .map_err(|error| format!("`{local}` does not render: {error}"))?;
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
