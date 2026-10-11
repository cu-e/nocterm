//! Deterministic, non-fatal discovery of user files and managed extensions.
use crate::{
    Appearance, ExtensionInfo, FILE_LIMIT, ThemeDiagnostic, ThemeError, ThemeOperationError,
    ZedTheme, parse_family, valid_id,
};
use std::{
    collections::BTreeSet,
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
    pub error: ThemeDiagnostic,
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
}
/// Parses supplied documents in caller-defined precedence, without opening paths.
pub struct CatalogBuilder {
    entries: Vec<ThemeEntry>,
    packs: Vec<ExtensionInfo>,
    problems: Vec<ThemeProblem>,
    names: BTreeSet<String>,
}
impl CatalogBuilder {
    pub fn new() -> Self {
        Self {
            names: BTreeSet::from(["Nocterm Default".into()]),
            entries: Vec::new(),
            packs: Vec::new(),
            problems: Vec::new(),
        }
    }
    pub fn manifest(&mut self, path: &Path, id: &str, bytes: &[u8]) -> Option<ThemeSource> {
        let result = self.parse_manifest(id, bytes);
        match result {
            Ok(pack) => {
                let source = ThemeSource::Installed {
                    id: pack.id.clone(),
                    version: pack.version.clone(),
                };
                self.packs.push(pack);
                Some(source)
            }
            Err(error) => {
                self.problem(path, error.into());
                None
            }
        }
    }
    fn parse_manifest(&self, id: &str, bytes: &[u8]) -> Result<ExtensionInfo, ThemeError> {
        if bytes.len() > FILE_LIMIT {
            return Err(ThemeError::FileLimit);
        }
        if !valid_id(id) {
            return Err(ThemeError::Identity);
        }
        let pack: ExtensionInfo = toml::from_str(std::str::from_utf8(bytes)?)?;
        if pack.id != id {
            return Err(ThemeError::ManifestIdentity);
        }
        Ok(pack)
    }
    pub fn document(&mut self, path: &Path, source: ThemeSource, bytes: &[u8]) {
        match parse_family(bytes) {
            Ok(family) => {
                for theme in family.themes {
                    if !self.names.insert(theme.name.clone()) {
                        self.problem(path, ThemeError::Duplicate(theme.name).into());
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
            Err(error) => self.problem(path, error.into()),
        }
    }
    pub fn problem(&mut self, path: &Path, error: ThemeOperationError) {
        self.problems.push(ThemeProblem {
            file: path.into(),
            error: ThemeDiagnostic::new(error),
        });
    }
    pub fn finish(mut self) -> ThemeCatalog {
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
        ThemeCatalog {
            entries: self.entries,
            packs: self.packs,
            problems: self.problems,
        }
    }
}
#[cfg(test)]
mod tests;

impl Default for CatalogBuilder {
    fn default() -> Self {
        Self::new()
    }
}
