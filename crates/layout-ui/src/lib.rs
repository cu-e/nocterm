//! The editor of the status bar and the panels: which layout is used and where each item of it
//! sits, in one section of Appearance. Everything is moved by dragging it in a preview of the
//! bar, or through the item's menu.
//!
//! Owns: the section's view and its state (the layout shown, the selected item, undo and redo,
//! renaming a layout or a group and confirming a delete), the drags, menus and keys that edit
//! the bar (a drop names a place; the rules of the move stay in `nocterm-window-layout`), how
//! the bar preview fits its one line, and what a sent edit led to (the notice, the selection
//! that follows it, the history step).
//! Does not own: the rules of layouts and their edits (`nocterm-window-layout`), the write of
//! an edit (`nocterm_workspace::edit_layouts`, which refuses a `[layouts]` table that does not
//! read), what the plugins are or whether they are on (`nocterm-plugin-model`,
//! `nocterm-plugin-enablement`), the settings of a plugin (the Plugins page, which an item's
//! menu opens), the Settings host, or how the footer and the panels are drawn
//! (`nocterm-workspace`).
//! Ports used: `SettingsHandle` (the layouts and the enablement), `UserPluginFacts` (what the
//! host knows of a user plugin, in the tooltip of its items), `nocterm-workspace`
//! (`edit_layouts` and its `LayoutEditError`, the `OpenPlugins` action). Provided:
//! [`layout_section`], an `EmbeddedPageSpec` for the application to place in Appearance.
//! Settings section: none of its own; it edits `layouts` (window-layout) through the rules.
//! Globals: none.
//! Invariants: every change to the layouts goes through the pure `edit` and shows the result of
//! the write, never a guess; editing the layout in use changes the footer at once; editing a
//! built-in layout in use saves a custom copy that stays in use; history is dropped when
//! anything else changes the layouts.

mod commands;
mod dnd;
mod fit;
mod history;
mod keys;
mod menu;
mod parts;
mod presets;
mod preview;
mod rename;
mod section;
mod selection;
mod snapshot;
mod tray;
mod user_facts;

use gpui_kit::AppContext as _;
use nocterm_store::SettingsHandle;
use std::rc::Rc;

use nocterm_ui::{EmbeddedPageSpec, PluginSource};

pub use section::LayoutEditorSection;
pub use user_facts::{UserFacts, UserPluginFacts};

/// The section of Appearance that edits the status bar and the panels.
///
/// `facts` tells what the host knows of an installed user plugin; without it an item of a user
/// plugin shows its name and its plugin alone.
///
/// `legacy_swapped` is the frozen `workspace.swapped` flag of the earlier swap-sides choice, which
/// decides what the built-in layout in use is while nothing else has been chosen.
pub fn layout_section(
    settings: SettingsHandle,
    source: Rc<dyn PluginSource>,
    facts: Option<Rc<dyn UserPluginFacts>>,
    legacy_swapped: bool,
) -> EmbeddedPageSpec {
    EmbeddedPageSpec::new("layout", "Status bar and panels", move |_, cx| {
        cx.new(|cx| {
            LayoutEditorSection::new(
                settings.clone(),
                source.clone(),
                facts.clone(),
                legacy_swapped,
                cx,
            )
        })
    })
}

#[cfg(test)]
mod tests;
