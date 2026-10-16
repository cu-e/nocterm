//! Explorer indexing and file-opening rules.
//! Owns: settings schemas, matching, argument substitution and sanitization.
//! Does not own: traversal, transfers, native paths or program execution.
//! Ports: SettingsSection. Settings: explorer. Globals: none.
//! Invariant: ordered first matching opener and literal arguments preserve file policy.
mod settings;
pub use settings::*;
