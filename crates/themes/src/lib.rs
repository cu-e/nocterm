//! Zed colour themes without a dependency on the application or GUI.
mod catalog;
mod install;
mod registry;
mod zed;

pub use catalog::{ThemeCatalog, ThemeDirs, ThemeEntry, ThemeProblem, ThemeSource};
pub use install::{install_archive, uninstall, valid_id};
pub use registry::{ExtensionInfo, ThemeRegistry, ZedRegistry};
pub use zed::{Appearance, Player, ThemeFamily, ZedTheme, parse_color, parse_family};

/// Maximum size of a single original theme file.
pub const FILE_LIMIT: usize = 8 * 1024 * 1024;
/// Maximum compressed size of an extension archive.
pub const ARCHIVE_LIMIT: usize = 16 * 1024 * 1024;

/// An invalid theme, registry response, archive, or filesystem operation.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}
