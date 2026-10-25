//! The editor of the status bar and the panels: where each item sits, which layout is used, and
//! the settings of the selected item, in one section of Appearance.
//!
//! Owns: the section's view and its state (the layout shown, the selected item, undo and redo,
//! rename and delete confirmation), the keys and the drags that move items (a drop names a
//! place; the rules of the move stay in `nocterm-window-layout`), how the bar preview fits its
//! one line, and what a sent edit led to (the notice, the selection that follows it, the history
//! step).
//! Does not own: the rules of layouts and their edits (`nocterm-window-layout`), the write of
//! an edit (`nocterm_workspace::edit_layouts`, which refuses a `[layouts]` table that does not
//! read), what the plugins are or whether they are on (`nocterm-plugin-model`,
//! `nocterm-plugin-enablement`), the form of a plugin's settings (`nocterm-ui`), the Settings
//! host, or how the footer and the panels are drawn (`nocterm-workspace`).
//! Ports used: `SettingsHandle` (the layouts and the enablement), `PluginSettingsDirectory`
//! (the settings of the selected item), `nocterm-workspace` (`edit_layouts` and its
//! `LayoutEditError`, the `OpenPlugins` action). Provided: [`layout_section`], an
//! `EmbeddedPageSpec` for the application to place in Appearance.
//! Settings section: none of its own; it edits `layouts` (window-layout) through the rules.
//! Globals: none.
//! Invariants: every change to the layouts goes through the pure `edit` and shows the result of
//! the write, never a guess; editing the layout in use changes the footer at once; editing a
//! built-in layout in use saves a custom copy that stays in use; history is dropped when
//! anything else changes the layouts; the settings form of an item is never part of undo.

mod commands;
mod dnd;
mod fit;
mod history;
mod inspector;
mod keys;
mod menu;
mod parts;
mod presets;
mod preview;
mod rename;
mod row;
mod section;
mod selection;
mod snapshot;
mod tray;
mod zones;

use gpui_kit::AppContext as _;
use nocterm_store::SettingsHandle;
use std::rc::Rc;

use nocterm_ui::{EmbeddedPageSpec, PluginSettingsDirectory, PluginSource};

pub use section::LayoutEditorSection;

/// The section of Appearance that edits the status bar and the panels.
///
/// `legacy_swapped` is the frozen `workspace.swapped` flag of the earlier swap-sides choice, which
/// decides what the built-in layout in use is while nothing else has been chosen.
pub fn layout_section(
    settings: SettingsHandle,
    source: Rc<dyn PluginSource>,
    directory: PluginSettingsDirectory,
    legacy_swapped: bool,
) -> EmbeddedPageSpec {
    EmbeddedPageSpec::new("layout", "Status bar and panels", move |_, cx| {
        cx.new(|cx| {
            LayoutEditorSection::new(
                settings.clone(),
                source.clone(),
                directory.clone(),
                legacy_swapped,
                cx,
            )
        })
    })
}

#[cfg(test)]
mod tests;
