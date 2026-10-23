//! Explorer indexing and file-opening rules.
//! Owns: settings schemas, matching, argument substitution and sanitization.
//! Does not own: traversal, transfers or program execution.
//! Directory paths, completion rules and narrow read-only capabilities are pure values;
//! callers dispatch readers off the UI thread and retain their original source.
//! Ports: SettingsSection, DirectoryRead, DirectoryReaders. Settings: explorer. Globals: none.
//! Invariant: ordered first matching opener and literal arguments preserve file policy.
mod settings;
pub use settings::*;

pub mod directory;
#[cfg(feature = "test-support")]
pub mod testing;
