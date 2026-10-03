//! The bridge between nocterm's look and the UI toolkit.
//!
//! Design tokens (`nocterm-design`) and user settings (`nocterm-settings`)
//! know nothing about GPUI. This crate makes them application globals that
//! every view reads ([`ActiveDesign`], [`ActiveSettings`]), and projects them
//! onto the component library's theme, so standard components follow the
//! tokens without any view repeating a colour or a size.

mod design;
mod drag_preview;
mod icons;
pub mod notice;
mod session_options;
mod settings;
pub use session_options::SessionOptionsEditor;
mod terminal_style;
mod theme;

pub use design::{ActiveDesign, Design};
pub use drag_preview::DragPreview;
pub use icons::{Assets, IconName};
pub use settings::{ActiveSettings, SettingsStore, save_settings, update_settings};
pub use terminal_style::{TerminalStyle, hsla};
pub use theme::{apply_theme, unknown_color_names};

pub use nocterm_design::{Color, DesignTokens};

use gpui_kit::App;

/// Installs the design tokens and settings, themes the components after
/// them, and keeps the theme in step when either changes.
///
/// Call it after `gpui_kit::init`.
pub fn init(tokens: DesignTokens, settings: SettingsStore, cx: &mut App) {
    for name in unknown_color_names(&tokens) {
        tracing::warn!(%name, "design tokens name a colour the component theme does not have");
    }

    cx.set_global(Design::new(tokens));
    cx.set_global(settings);
    settings::init(cx);
    apply_theme(cx);

    cx.observe_global::<Design>(apply_theme).detach();
    cx.observe_global::<SettingsStore>(apply_theme).detach();
}
