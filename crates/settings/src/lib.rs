//! User settings: typed sections of `settings.toml` and the file itself.
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
