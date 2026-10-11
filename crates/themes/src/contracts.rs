//! Blocking library ports. Callers dispatch them on their executor.
use crate::{Appearance, ThemeCatalog, ThemeOperationError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub download_count: u64,
}
pub trait ThemeRegistry: Send + Sync {
    fn search(&self, query: &str) -> Result<Vec<ExtensionInfo>, ThemeOperationError>;
}
pub trait ThemeLibrary: Send + Sync {
    fn scan(&self) -> ThemeCatalog;
    fn install(&self, extension: &ExtensionInfo) -> Result<ThemeChange, ThemeOperationError>;
    fn uninstall(&self, id: &str) -> Result<ThemeChange, ThemeOperationError>;
    fn prepare_user_directory(&self) -> Result<PathBuf, ThemeOperationError>;
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeMutation {
    Install(String),
    Uninstall(String),
}
#[derive(Debug, Clone)]
pub struct ThemeChange {
    pub operation: ThemeMutation,
    pub catalog: ThemeCatalog,
    pub removed: Vec<(String, Appearance)>,
}
