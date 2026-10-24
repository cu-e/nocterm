//! The host-rendered settings form of a plugin and the port it reads and writes through.
//!
//! A plugin declares its settings as a [`SettingsSchema`](nocterm_plugin_model::SettingsSchema).
//! [`PluginSettingsForm`] draws the controls for it and sends each change as a
//! [`FieldEdit`](nocterm_plugin_model::FieldEdit) to a [`PluginSettingsSource`], which applies
//! it to the value current at write time. [`SectionBinding`] is the system implementation over
//! the owner's own settings section; [`PluginSettingsDirectory`] finds the source of a plugin.
//!
//! Text values are data: the form shows them as literal text in an ordinary single-line input,
//! never masked, and never interprets them.

mod binding;
mod controls;
mod directory;
#[cfg(any(test, feature = "test-support"))]
mod fake;
mod form;
mod source;

pub use binding::SectionBinding;
pub use directory::PluginSettingsDirectory;
#[cfg(any(test, feature = "test-support"))]
pub use fake::FakePluginSettings;
pub use form::{FormFrame, FormScope, PluginSettingsForm};
pub use source::{PluginSettingsError, PluginSettingsSource};

#[cfg(test)]
mod binding_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_tests;
