//! The bridge between nocterm's look and the UI toolkit.
//!
//! Design tokens (`nocterm-design`) and user settings (`nocterm-settings`)
//! know nothing about GPUI. This crate exposes the installed settings handle
//! and projects them onto the component library's theme. [`ActiveDesign`] remains
//! a compatibility paint accessor for presentation callers; shared
//! terminal styling accepts supplied settings; ThemeUi retains the original policy
//! and paints Design changes without resolving back into Design.
//!
//! Owns: Appearance views, theme projections, shared controls, directory editing/selection,
//! checked text receipts, layout, the searchable choice picker, and the icons and column widths of
//! plugin contributions.
//! Does not own: ordered snapshot workers or shutdown coordination (`nocterm-store`).
//! Ports used: `Persisted`, `ShutdownCoordinator`, read-only `DirectoryRead`. Ports provided: UI helpers,
//! explicit settings-based terminal styling, SettingsTextField and the installed settings handle.
//! Settings section: appearance. Terminal styling uses the terminal-model domain schema.
//! Globals: Design, Notices.
//! Imported themes use one retained ThemeUi projection over ThemesHandle and SettingsHandle.
//! Design is paint data only; OS callbacks apply the retained projection.
//! Feature policy schemas are registered by their owning service.
//! Invariants: layout edits remain visible on persistence failure; text drafts retain original checked receipts.
//! Text errors stay typed until display; close/deactivation synchronously admit distinct literals.

mod animations;
mod contribution;
mod design;
pub mod directory;
mod drag_preview;
mod floating;
pub mod form;
mod icons;
mod layout;
pub mod notice;
mod sections;
mod session_options;
mod settings_page;
mod settings_text_field;
pub use session_options::{SessionOptionsEditor, SessionOptionsInputError};
pub use settings_page::{SettingsPage, SettingsPageHandle, SettingsPageSpec};
pub use settings_text_field::SettingsTextField;
mod terminal_style;
mod theme;
mod themes;

pub use animations::thinking_indicator;
pub use contribution::{contribution_icon, width_range};
pub use design::{ActiveDesign, Design};
pub use drag_preview::{DragPreview, DragSource};
pub use icons::{Assets, IconName, agent_icon};
pub use layout::{LayoutHandle, LayoutMemory};
pub use nocterm_store::StoreError as SettingsError;
pub use sections::{
    AppearanceMode, AppearanceSettings, CARD_GAP_RANGE, CARD_RADIUS_RANGE, UiLayout,
};
pub use terminal_style::{TerminalStyle, hsla};
pub use theme::unknown_color_names;
pub use themes::ThemeUi;

pub use nocterm_design::{Color, DesignTokens};

use gpui_kit::App;

/// Initializes design and component paint with the supplied installed settings.
/// Retain [`ThemeUi`] to keep the projection in step when either changes.
///
/// Call it after `gpui_kit::init`. The returned witness is what the features
/// that read settings take, so calling them first does not compile.
pub fn init(
    tokens: DesignTokens,
    settings: nocterm_store::SettingsHandle,
    cx: &mut App,
) -> UiReady {
    for name in unknown_color_names(&tokens) {
        tracing::warn!(%name, "design tokens name a colour the component theme does not have");
    }

    directory::install_bindings(cx);
    cx.set_global(Design::new(tokens));
    settings.register::<crate::AppearanceSettings>(cx);
    settings.register::<crate::TerminalSettings>(cx);
    let appearance = settings.setting::<crate::AppearanceSettings>(cx).clone();
    theme::apply_theme_for(&appearance, cx);
    UiReady { settings }
}

/// Proof that [`init`] has initialized design and retained the supplied settings.
/// Features receive the explicit dependency through this witness.
#[derive(Clone)]
pub struct UiReady {
    settings: nocterm_store::SettingsHandle,
}

impl UiReady {
    pub fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
}

pub use nocterm_terminal_model::{
    ClipboardWritePolicy, CursorShape, FONT_SIZE_RANGE, LINE_HEIGHT_RANGE, SCROLLBACK_RANGE,
    TerminalSettings,
};

mod settings_page_context;
pub use settings_page_context::{SettingsPageContext, SettingsPageFeedback};
mod settings_field;
pub mod settings_input;
pub use settings_field::FieldPresentation;

mod appearance_page;
mod choice_picker;
pub use appearance_page::appearance_settings_page;
pub use appearance_page::commands::{open_scheme_picker, open_theme_picker};
pub use choice_picker::{PickerOption, open_choice_picker};

#[cfg(test)]
mod settings_initialization_tests;
