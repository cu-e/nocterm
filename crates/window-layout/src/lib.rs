//! The window shell's layout presets as plain data and pure rules.
//!
//! A preset arranges the contributions of the plugin catalog: the three zones of the status
//! bar, the panel groups and their members, the tray of contributions that are registered but
//! not placed, and the panels that open with the layout. [`resolve`] checks a preset against a
//! [`PluginSet`](nocterm_plugin_model::PluginSet) (the system catalog and the installed user
//! plugins) and returns the arrangement the shell draws; the plugins it is told are disabled keep their entries in the preset and drop out of
//! the arrangement. The built-in [`BuiltIn::Default`] is the empty preset, resolved from the
//! placement hints of the contributions, and [`BuiltIn::Swapped`] is the same arrangement with
//! every group on the other side.
//!
//! The user's presets are the `[layouts]` settings section, [`LayoutSettings`]. [`select`] picks
//! the layout to show, including the legacy swap-sides flag, and [`transition`] tells a window
//! whether a change of the section switched layouts, edited the selected one or changed nothing
//! it shows. [`edit`] changes the presets: the first edit of a built-in layout copies it into a
//! preset called "Custom" and selects it, the entry operations move, group, separate, remove
//! and restore what the bar holds, and the undo of an edit restores the earlier list of presets.
//!
//! Owns: bar zones, groups and separators, the tray, default-open panels, the built-in
//! Default and Swapped sides, resolution against the catalog, the `[layouts]` section and its
//! sanitizing, selection, the switch-or-edit transition, name validation and the edit operations
//! with their pinned and tray rules.
//! Does not own: which plugins exist or are disabled (the caller passes the disabled set),
//! panel state, widths, rendering, settings storage.
//! Ports used/provided: none; it reads `nocterm-plugin-model` and defines a section for
//! `nocterm-settings`. Settings section: layouts. Globals: none.
//! Invariants: built-ins are never stored and always resolve; unknown ids and the ids of
//! disabled plugins and user contributions the user has not approved for placement survive in the
//! preset but never render, open or get added by hints; a
//! contribution is placed at most once; at most one open panel per group; exactly one group is
//! primary, the catalog's (a preset's own `primary` flag is ignored); pinned widgets are always
//! placed (pinning has no effect on a panel); every panel has a hosting group, placed or not;
//! sanitizing leaves every stored preset with a unique usable id and name; an edit writes the
//! preset out as it resolves with every plugin enabled, moves ids of the bar the catalog does not
//! know to the tray, never removes a pinned widget and leaves the settings untouched when it
//! fails. No I/O.

mod builtin;
mod edit;
mod name;
mod preset;
mod resolve;
mod resolved;
mod settings;
mod transition;
mod wire;

pub use builtin::BuiltIn;
pub use edit::{EditError, EditOp, EditOutcome, EditTarget, Limit, Slot, Subject, edit};
pub use name::{MAX_NAME_CHARS, NameError, validate_name};
pub use preset::{Bar, BarItem, GroupEntry, LayoutPreset, PresetId};
pub use resolve::resolve;
pub use resolved::{ResolvedGroup, ResolvedItem, ResolvedLayout, ResolvedMember, solo_group_id};
pub use settings::{LayoutSettings, MAX_GROUP_MEMBERS, MAX_PRESETS, MAX_ZONE_ITEMS, select};
pub use transition::{LayoutTransition, transition};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod settings_tests;

#[cfg(test)]
mod disabled_tests;

#[cfg(test)]
mod edit_tests;

#[cfg(test)]
mod edit_entry_tests;
#[cfg(test)]
mod edit_grouping_tests;
#[cfg(test)]
mod user_plugin_tests;
