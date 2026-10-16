//! The bridge between nocterm's look and the UI toolkit.
//!
//! Design tokens (`nocterm-design`) and user settings (`nocterm-settings`)
//! know nothing about GPUI. This crate exposes the installed settings handle
//! and projects them onto the component library's theme. [`ActiveDesign`] and
//! [`SettingsExt`] remain compatibility accessors for presentation callers; shared
//! terminal styling accepts supplied settings; ThemeUi retains the original policy
//! and paints Design changes without resolving back into Design.
//!
//! Owns: theme projections, shared assets/forms, checked native text editing, settings compatibility and layout.
//! Does not own: ordered snapshot workers or shutdown coordination (`nocterm-store`).
//! Ports used: `Persisted`, `ShutdownCoordinator`. Ports provided: UI helpers,
//! explicit settings-based terminal styling, SettingsTextField and the installed settings handle.
//! Settings sections: appearance, terminal.
//! Globals: Design, SettingsStore, Notices.
//! Imported themes use one retained ThemeUi projection over ThemesHandle and SettingsHandle.
//! Design is paint data only; OS callbacks apply the retained projection.
//! Feature policy schemas are registered by their owning service.
//! Invariants: layout edits remain visible on persistence failure; text drafts retain original checked receipts.
//! Text errors stay typed until display; close/deactivation synchronously admit distinct literals.

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
mod settings_page;
mod settings_text_field;
pub use session_options::{SessionOptionsEditor, SessionOptionsInputError};
pub use settings_page::{SettingsPage, SettingsPageHandle, SettingsPageSpec};
pub use settings_text_field::SettingsTextField;
mod terminal_style;
mod theme;
mod themes;

pub use design::{ActiveDesign, Design};
pub use drag_preview::{DragPreview, DragSource};
pub use icons::{Assets, IconName, agent_icon};
pub use layout::{LayoutHandle, LayoutMemory};
pub use nocterm_store::StoreError as SettingsError;
pub use sections::{
    AppearanceMode, AppearanceSettings, CARD_GAP_RANGE, CARD_RADIUS_RANGE, ClipboardWritePolicy,
    CursorShape, FONT_SIZE_RANGE, LINE_HEIGHT_RANGE, SCROLLBACK_RANGE, TerminalSettings, UiLayout,
};
pub use settings::{SettingsExt, SettingsStore, edit_settings, register_setting};
pub use terminal_style::{TerminalStyle, hsla};
pub use theme::unknown_color_names;
pub use themes::ThemeUi;

pub use nocterm_design::{Color, DesignTokens};

use gpui_kit::App;

/// Installs the design tokens and settings, themes the components after
/// them. Retain [`ThemeUi`] to keep the projection in step when either changes.
///
/// Call it after `gpui_kit::init`. The returned witness is what the features
/// that read settings take, so calling them first does not compile.
pub fn init(tokens: DesignTokens, settings: SettingsStore, cx: &mut App) -> UiReady {
    for name in unknown_color_names(&tokens) {
        tracing::warn!(%name, "design tokens name a colour the component theme does not have");
    }

    cx.set_global(Design::new(tokens));
    cx.set_global(settings);
    let settings = settings::init(cx);
    settings.register::<crate::AppearanceSettings>(cx);
    settings.register::<crate::TerminalSettings>(cx);
    let appearance = settings.setting::<crate::AppearanceSettings>(cx).clone();
    theme::apply_theme_for(&appearance, cx);
    UiReady { settings }
}

/// Proof that [`init`] has run: the design and the settings store are
/// installed. It retains the explicit settings dependency for composition.
#[derive(Clone)]
pub struct UiReady {
    settings: nocterm_store::SettingsHandle,
}

impl UiReady {
    pub fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
    /// The witness for an app [`init`] already ran in, for code handed the
    /// app rather than the witness, such as a test re-initializing a feature.
    pub fn installed(cx: &App) -> Option<Self> {
        cx.try_global::<SettingsStore>()?
            .handle()
            .cloned()
            .map(|settings| Self { settings })
    }
}
