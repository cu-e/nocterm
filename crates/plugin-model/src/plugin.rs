//! The declaration of a system plugin.

use crate::{PluginId, SettingsSchema, SystemContribution};

/// A plugin compiled into Nocterm and registered by its feature crate.
///
/// Owners declare it as `pub static PLUGIN: SystemPlugin` in their `plugin` module. It is a
/// `static` because the catalog keeps `&'static` references to it, and a reference to a
/// `const` holding a `Cow` id is not promoted to `'static`. Core owners declare
/// `pub const CORE: &[SystemContribution]` instead, which is already a `'static` reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemPlugin {
    pub id: PluginId,
    pub name: &'static str,
    pub description: &'static str,
    /// A required plugin cannot be switched off.
    pub required: bool,
    pub contributions: &'static [SystemContribution],
    /// The key of the owner's `SettingsSection` that stores the plugin's settings.
    pub settings_section: Option<&'static str>,
    /// The settings the plugin exposes. A non-empty schema needs a `settings_section`.
    pub settings: SettingsSchema,
    /// The id of the owner's own Settings page, when the plugin has more than a form. The page
    /// is either nested under Plugins or a top-level page of its own, as the AI page is; it
    /// opens whether or not the plugin is enabled.
    pub settings_page: Option<&'static str>,
}
