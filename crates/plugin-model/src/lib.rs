//! What a plugin is and what the window shell can host.
//!
//! A plugin is a named unit that owns contributions: panels in the side groups and widgets in
//! the status bar. This crate holds the plain data every owner declares (a `static` plugin,
//! and core contributions as a `const`), the placement hints the shell reads, and the checked
//! [`PluginCatalog`] of core contributions (shell functions that are not plugins and are never
//! disabled) and system plugins.
//!
//! Owns: id syntax, descriptors and their placement hints, group specs, icons, width classes,
//! catalog validation and contribution ownership (core or plugin).
//! Does not own: GPUI types, rendering, panel state, layouts, settings storage, enablement.
//! Ports used/provided: none. Settings section: none. Globals: none.
//! Invariants: ids are unique and namespaced by their plugin; exactly one group is primary;
//! group hints name declared groups and only panels have one; panel traits exist exactly for
//! panels; order values are unique within a zone.
//! No internal dependencies; no I/O.

mod catalog;
mod contribution;
mod error;
mod id;
mod plugin;

pub use catalog::{Owner, PluginCatalog};
pub use contribution::{
    AutoPlace, ContributionKind, GroupSpec, Icon, PanelChrome, PanelTraits, Placement, Side,
    SystemContribution, WidthClass, Zone,
};
pub use error::{CatalogError, IdError};
pub use id::{ContributionId, GroupId, PluginId};
pub use plugin::SystemPlugin;

#[cfg(test)]
mod tests;
