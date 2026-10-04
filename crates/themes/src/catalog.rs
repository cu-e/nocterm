//! Deterministic, non-fatal discovery of user files and managed extensions.
use crate::{Appearance, ExtensionInfo, FILE_LIMIT, ThemeError, ZedTheme, parse_family, valid_id};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct ThemeDirs {
    pub user: PathBuf,
    pub installed: PathBuf,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeSource {
    User(PathBuf),
    Installed { id: String, version: String },
}
#[derive(Debug, Clone)]
pub struct ThemeEntry {
    pub name: String,
    pub appearance: Appearance,
    pub family: String,
    pub author: String,
    pub source: ThemeSource,
    pub theme: ZedTheme,
}
#[derive(Debug, Clone)]
pub struct ThemeProblem {
    pub file: PathBuf,
    pub error: String,
}
#[derive(Debug, Clone, Default)]
pub struct ThemeCatalog {
    entries: Vec<ThemeEntry>,
    packs: Vec<ExtensionInfo>,
    problems: Vec<ThemeProblem>,
}
impl ThemeCatalog {
    pub fn entries(&self) -> &[ThemeEntry] {
        &self.entries
    }
    pub fn packs(&self) -> &[ExtensionInfo] {
        &self.packs
    }
    pub fn problems(&self) -> &[ThemeProblem] {
        &self.problems
    }
    pub fn find(&self, name: &str, appearance: Appearance) -> Option<&ThemeEntry> {
        self.entries
            .iter()
            .find(|e| e.name == name && e.appearance == appearance)
    }
    pub fn load(dirs: &ThemeDirs) -> Self {
        let mut catalog = Self::default();
        let mut names = BTreeSet::from(["Nocterm Default".to_owned()]);
        for path in catalog.paths(&dirs.user) {
            if is_plain_file(&path) && path.extension().is_some_and(|x| x == "json") {
                catalog.file(&path, ThemeSource::User(path.clone()), &mut names);
            }
        }
        for directory in catalog.paths(&dirs.installed) {
            let Some(id) = directory
                .file_name()
                .and_then(|x| x.to_str())
                .filter(|x| valid_id(x))
            else {
                continue;
            };
            if !fs::symlink_metadata(&directory)
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            {
                continue;
            }
            let manifest_path = directory.join("extension.toml");
            let manifest = read_bounded(&manifest_path).and_then(|b| {
                let text =
                    std::str::from_utf8(&b).map_err(|e| ThemeError::Invalid(e.to_string()))?;
                toml::from_str::<ExtensionInfo>(text)
                    .map_err(|e| ThemeError::Invalid(e.to_string()))
            });
            match manifest {
                Ok(pack) if pack.id == id => {
                    let source = ThemeSource::Installed {
                        id: pack.id.clone(),
                        version: pack.version.clone(),
                    };
                    for path in catalog.paths(&directory.join("themes")) {
                        if is_plain_file(&path) && path.extension().is_some_and(|x| x == "json") {
                            catalog.file(&path, source.clone(), &mut names);
                        }
                    }
                    catalog.packs.push(pack);
                }
                Ok(_) => catalog.problem(&manifest_path, "manifest id does not match directory"),
                Err(e) => catalog.problem(&manifest_path, e.to_string()),
            }
        }
        catalog.entries.sort_by(|a, b| a.name.cmp(&b.name));
        catalog
    }
    fn paths(&mut self, directory: &Path) -> Vec<PathBuf> {
        match fs::read_dir(directory) {
            Ok(entries) => {
                let mut paths = Vec::new();
                for entry in entries {
                    match entry {
                        Ok(e) => paths.push(e.path()),
                        Err(e) => self.problem(directory, e.to_string()),
                    }
                }
                paths.sort();
                paths
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                self.problem(directory, e.to_string());
                Vec::new()
            }
        }
    }
    fn file(&mut self, file: &Path, source: ThemeSource, names: &mut BTreeSet<String>) {
        match read_bounded(file).and_then(|b| parse_family(&b)) {
            Ok(family) => {
                for theme in family.themes {
                    if !names.insert(theme.name.clone()) {
                        self.problem(
                            file,
                            format!("duplicate or reserved theme name: {}", theme.name),
                        );
                        continue;
                    }
                    self.entries.push(ThemeEntry {
                        name: theme.name.clone(),
                        appearance: theme.appearance,
                        family: family.name.clone(),
                        author: family.author.clone(),
                        source: source.clone(),
                        theme,
                    });
                }
            }
            Err(e) => self.problem(file, e.to_string()),
        }
    }
    fn problem(&mut self, file: &Path, error: impl Into<String>) {
        self.problems.push(ThemeProblem {
            file: file.into(),
            error: error.into(),
        });
    }
}
fn is_plain_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
}
fn read_bounded(path: &Path) -> Result<Vec<u8>, ThemeError> {
    if !is_plain_file(path) {
        return Err(ThemeError::Invalid("expected a regular file".into()));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(FILE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > FILE_LIMIT {
        return Err(ThemeError::Invalid("file exceeds 8 MiB".into()));
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests;
