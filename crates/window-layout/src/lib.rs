//! The window shell's layout presets as plain data and pure rules.
//!
//! A preset arranges the contributions of the plugin catalog: the three zones of the status
//! bar, the panel groups and their members, the tray of contributions that are registered but
//! not placed, and the panels that open with the layout. [`resolve`] checks a preset against a
//! [`PluginCatalog`](nocterm_plugin_model::PluginCatalog) and returns the arrangement the shell
//! draws. The built-in [`BuiltIn::Default`] is the empty preset, resolved from the placement
//! hints of the contributions, and [`BuiltIn::Swapped`] is the same arrangement with every
//! group on the other side.
//!
//! Owns: bar zones, groups and separators, the tray, default-open panels, the built-in
//! Default and Swapped sides, and resolution against the catalog.
//! Does not own: which plugins exist or are enabled, panel state, widths, rendering, settings
//! storage.
//! Ports used/provided: none; it reads `nocterm-plugin-model`. Settings section: none yet.
//! Globals: none.
//! Invariants: built-ins are never stored and always resolve; unknown ids survive in the preset
//! but never render; a contribution is placed at most once; at most one open panel per group;
//! exactly one group is primary, the catalog's (a preset's own `primary` flag is ignored);
//! pinned widgets are always placed (pinning has no effect on a panel); every panel has a
//! hosting group, placed or not. No I/O.

mod builtin;
mod preset;
mod resolve;
mod resolved;

pub use builtin::BuiltIn;
pub use preset::{Bar, BarItem, GroupEntry, LayoutPreset, PresetId};
pub use resolve::resolve;
pub use resolved::{ResolvedGroup, ResolvedItem, ResolvedLayout, ResolvedMember};

#[cfg(test)]
mod tests;
