//! Deterministic, non-fatal discovery of user files and managed extensions.
use crate::{
    Appearance, ExtensionInfo, FILE_LIMIT, ThemeDiagnostic, ThemeError, ThemeOperationError,
    ZedTheme, parse_family, valid_id,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
const MAX_VARIANTS: usize = 2_048;
const MAX_PACKS: usize = 256;
const MAX_PROBLEMS: usize = 128;
const MAX_CONTENT_BYTES: usize = 32 * 1024 * 1024;

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
    content_bytes: usize,
}
impl CatalogBuilder {
    pub fn new() -> Self {
        Self {
            names: BTreeSet::from(["Nocterm Default".into()]),
            entries: Vec::new(),
            packs: Vec::new(),
            problems: Vec::new(),
            content_bytes: 0,
        }
    }
    pub fn manifest(&mut self, path: &Path, id: &str, bytes: &[u8]) -> Option<ThemeSource> {
        if self.packs.len() >= MAX_PACKS {
            self.problem(
                path,
                ThemeOperationError::DiscoveryLimit {
                    resource: "packs",
                    max: MAX_PACKS,
                },
            );
            return None;
        }
        let result = self.parse_manifest(id, bytes);
        match result {
            Ok(pack) => {
                let bytes = pack.id.len()
                    + pack.version.len()
                    + pack.name.len()
                    + pack.authors.iter().map(String::len).sum::<usize>()
                    + pack.description.as_ref().map_or(0, String::len)
                    + pack.repository.as_ref().map_or(0, String::len);
                if !self.admit_content(path, bytes) {
                    return None;
                }
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
                    if self.entries.len() >= MAX_VARIANTS {
                        self.problem(
                            path,
                            ThemeOperationError::DiscoveryLimit {
                                resource: "variants",
                                max: MAX_VARIANTS,
                            },
                        );
                        break;
                    }
                    let bytes = family.name.len()
                        + family.author.len()
                        + theme.name.len() * 3
                        + theme.colors.keys().map(String::len).sum::<usize>()
                        + match &source {
                            ThemeSource::User(path) => path.as_os_str().len(),
                            ThemeSource::Installed { id, version } => id.len() + version.len(),
                        };
                    if !self.admit_content(path, bytes) {
                        break;
                    }
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
    fn admit_content(&mut self, path: &Path, bytes: usize) -> bool {
        if bytes > MAX_CONTENT_BYTES.saturating_sub(self.content_bytes) {
            self.problem(
                path,
                ThemeOperationError::DiscoveryLimit {
                    resource: "retained content bytes",
                    max: MAX_CONTENT_BYTES,
                },
            );
            return false;
        }
        self.content_bytes += bytes;
        true
    }
    pub fn problem(&mut self, path: &Path, error: ThemeOperationError) {
        if self.problems.len() > MAX_PROBLEMS {
            return;
        }
        let error = if self.problems.len() == MAX_PROBLEMS {
            ThemeOperationError::DiscoveryLimit {
                resource: "diagnostics",
                max: MAX_PROBLEMS,
            }
        } else {
            error
        };
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
