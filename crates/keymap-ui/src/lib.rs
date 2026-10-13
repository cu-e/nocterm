//! The Keymap settings page: every command with its shortcuts, searchable,
//! rebindable by pressing the new keys, and resettable to the defaults.
//!
//! The page edits through [`nocterm_keymap::Keymap`], which owns the
//! bindings and the user's file; it knows nothing about the features whose
//! commands it lists.
//!
//! Owns: keymap search, shortcut recording, conflict presentation and the settings page.
//! Does not own: binding storage, effective keymap installation or feature commands.
//! Ports used: `Keymap`; provided: UI `SettingsPageSpec`.
//! Settings section: none (bindings use the keymap file). Globals: none.
//! Invariants: edits go through the keymap owner; the page does not rewrite other bindings.
mod render;
mod rows;
mod view;

use gpui_kit::AppContext as _;
use nocterm_ui::SettingsPageSpec;

pub use view::KeymapView;

/// Supplies the lazy Keymap page to the application Settings host.
pub fn settings_page() -> SettingsPageSpec {
    SettingsPageSpec::new("keymap", "Keymap", |window, cx| {
        cx.new(|cx| KeymapView::new(window, cx))
    })
    .with_icon(nocterm_ui::IconName::Keyboard)
}

#[cfg(test)]
mod tests;
