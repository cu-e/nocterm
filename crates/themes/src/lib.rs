//! Pure Zed theme formats, palette mapping and deterministic catalog rules.
//!
//! Owns: supplied-document parsing, identities and library/registry contracts.
//! Does not own: discovery, archives, files, HTTP, scheduling or views.
//! Ports provided: ThemeLibrary, ThemeRegistry. Ports used: none.
//! Settings section: none. Globals: none.
//! Invariants: unique variant names; Nocterm Default is reserved.
mod catalog;
mod contracts;
mod error;
mod identity;
mod zed;

pub use catalog::{CatalogBuilder, ThemeCatalog, ThemeDirs, ThemeEntry, ThemeProblem, ThemeSource};
pub use contracts::{ExtensionInfo, ThemeChange, ThemeLibrary, ThemeMutation, ThemeRegistry};
pub use error::{ThemeDiagnostic, ThemeError, ThemeOperationError};
pub use identity::valid_id;
pub use zed::{Appearance, Player, ThemeFamily, ZedTheme, parse_color, parse_family};

/// Maximum size of a single original theme file.
pub const FILE_LIMIT: usize = 8 * 1024 * 1024;
/// Maximum compressed size of an extension archive.
pub const ARCHIVE_LIMIT: usize = 16 * 1024 * 1024;

#[cfg(feature = "test-support")]
pub mod test_support;
