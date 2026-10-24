//! What a plugin is and what the window shell can host.
//!
//! A plugin is a named unit that owns contributions: panels in the side groups and widgets in
//! the status bar. This crate holds the plain data every owner declares (a `static` plugin,
//! and core contributions as a `const`), the placement hints the shell reads, and the checked
//! [`PluginCatalog`] of core contributions (shell functions that are not plugins and are never
//! disabled) and system plugins, and the declarative settings schema a plugin exposes.
//!
//! Owns: id syntax, descriptors and their placement hints, group specs, icons, width classes,
//! catalog validation, contribution ownership (core or plugin), schema validation, text
//! sanitizing, pure field edits, the user tier (`user-` plugin ids, `UserPlugin`,
//! `UserContribution` and its clamp), borrowed `PluginRef`/`ContributionRef` views over both
//! tiers and the resolution of stored user settings against a versioned schema.
//! Does not own: GPUI types, rendering, panel state, layouts, settings storage, enablement.
//! Ports used/provided: none. Settings section: none. Globals: none.
//! Invariants: ids are unique and namespaced by their plugin; exactly one group is primary;
//! group hints name declared groups and only panels have one; panel traits exist exactly for
//! panels; order values are unique within a zone; settings schemas stay within their limits and
//! name a settings section; system plugins never use the reserved `user-` prefix; a user
//! contribution is only built by its clamp, so it has no built-in privilege.
//! `no_std` with `alloc`, so file, network, process and environment access is unreachable.
//! No internal dependencies; no I/O.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod catalog;
mod contribution;
mod error;
mod id;
mod plugin;
mod schema;
mod user;
mod view;

pub use catalog::{Owner, PluginCatalog};
pub use contribution::{
    AutoPlace, ContributionKind, GroupSpec, Icon, PanelChrome, PanelTraits, Placement, Side,
    SystemContribution, WidthClass, Zone,
};
pub use error::{CatalogError, IdError};
pub use id::{ContributionId, GroupId, PluginId, USER_PREFIX};
pub use plugin::SystemPlugin;
pub use schema::{
    ChoiceOption, FieldEdit, FieldError, FieldGroup, FieldIssue, FieldKind, FieldSpec, MAX_FIELDS,
    MAX_GROUPS, MAX_HELP, MAX_KEY, MAX_LABEL, MAX_OPTIONS, MAX_TEXT_LEN, SchemaError, SchemaLimit,
    SettingValue, SettingValues, SettingsSchema, apply, bounded, resolve, sanitize_text,
    toggle_choice,
};
pub use user::{PluginTier, UserContribution, UserPlugin, UserPluginError};
pub use view::{ContributionRef, PluginRef};

#[cfg(test)]
mod tests;
