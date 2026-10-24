//! The declaration of a system plugin.

use crate::{PluginId, SystemContribution};

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
}
