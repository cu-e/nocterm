//! The bridge between nocterm's look and the UI toolkit.
//!
//! Design tokens (`nocterm-design`) and user settings (`nocterm-settings`)
//! know nothing about GPUI. This crate makes them application globals that
//! every view reads ([`ActiveDesign`], [`SettingsExt`]), and projects them
//! onto the component library's theme, so standard components follow the
//! tokens without any view repeating a colour or a size.

mod ai;
mod design;
mod drag_preview;
mod floating;
pub mod form;
mod icons;
mod layout;
pub mod notice;
mod sections;
mod session_options;
mod settings;
pub use session_options::SessionOptionsEditor;
mod terminal_style;
mod theme;
mod themes;

pub use ai::{ActiveAi, observe_ai_enabled};
pub use design::{ActiveDesign, Design};
pub use drag_preview::{DragPreview, DragSource};
pub use icons::{Assets, IconName, agent_icon};
pub use layout::LayoutMemory;
pub use sections::{
    AppearanceMode, AppearanceSettings, CARD_GAP_RANGE, CARD_RADIUS_RANGE, ClipboardWritePolicy,
    CursorShape, FONT_SIZE_RANGE, LINE_HEIGHT_RANGE, SCROLLBACK_RANGE, TerminalSettings, UiLayout,
};
pub use settings::{SettingsExt, SettingsStore, edit_settings, register_setting};
pub use terminal_style::{TerminalStyle, hsla};
pub use theme::{apply_theme, unknown_color_names};
pub use themes::{ActiveThemes, Themes, init_themes, reload_themes};

pub use nocterm_design::{Color, DesignTokens};

use gpui_kit::App;

/// Installs the design tokens and settings, themes the components after
/// them, and keeps the theme in step when either changes.
///
/// Call it after `gpui_kit::init`. The returned witness is what the features
/// that read settings take, so calling them first does not compile.
pub fn init(tokens: DesignTokens, settings: SettingsStore, cx: &mut App) -> UiReady {
    for name in unknown_color_names(&tokens) {
        tracing::warn!(%name, "design tokens name a colour the component theme does not have");
    }

    cx.set_global(Design::new(tokens));
    cx.set_global(settings);
    settings::init(cx);
    register_setting::<crate::AppearanceSettings>(cx);
    register_setting::<crate::TerminalSettings>(cx);
    register_setting::<nocterm_ai::AiSettings>(cx);
    apply_theme(cx);

    cx.observe_global::<Design>(apply_theme).detach();
    cx.observe_global::<SettingsStore>(apply_theme).detach();
    UiReady(())
}

/// Proof that [`init`] has run: the design and the settings store are
/// installed. It costs nothing at run time.
#[derive(Clone, Copy, Debug)]
pub struct UiReady(());

impl UiReady {
    /// The witness for an app [`init`] already ran in, for code handed the
    /// app rather than the witness, such as a test re-initializing a feature.
    pub fn installed(cx: &App) -> Option<Self> {
        cx.has_global::<SettingsStore>().then_some(Self(()))
    }
}
