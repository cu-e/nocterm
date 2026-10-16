//! User settings: typed sections of `settings.toml` and the file itself.
//!
//! Owns: section registration, independent table decoding, protected roundtrips
//! and SettingsFile persistence through the shared core configuration mechanism.
//! Does not own: product section schemas, GPUI state, background writers or views.
//! Ports provided: SettingsSection. Ports used: none. Settings section: none.
//! Globals: none. Invariants: a broken section uses defaults and remains untouched
//! on save; unclaimed or broken tables retain the current file contents.
//!
//! This crate knows no feature. Each crate defines the sections it uses
//! ([`SettingsSection`]); the document reads every section on its own, so a
//! mistake in one leaves the others working.
//!
//! Settings are what a *user* chooses (which font, which cursor); design
//! tokens (`nocterm-design`) are what the *product* looks like by default.
//! A setting left unset falls back to the matching token.

mod document;
#[cfg(test)]
mod fixtures;
mod section;
mod store;

pub use document::{SectionError, SettingsDocument};
pub use section::{SettingsSection, clamp_f32, trim_unset};
pub use store::SettingsFile;

pub mod schema;
