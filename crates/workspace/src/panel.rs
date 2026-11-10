use gpui_kit::{AnyView, App, Entity, EntityId, FocusHandle, Focusable, Render, SharedString};
use nocterm_ui::IconName;

/// A view of the sidebar.
pub trait Panel: Render + Focusable {
    /// Shown above the panel and as the tooltip of its switcher icon.
    fn title(&self, cx: &App) -> SharedString;

    /// The panel's icon in the switcher strip at the foot of the sidebar.
    fn icon(&self, cx: &App) -> IconName;

    /// A short count beside the switcher icon, such as how many things in
    /// the panel need attention; `None` shows none. The footer is redrawn
    /// when that title, icon or badge changes.
    fn badge(&self, cx: &App) -> Option<SharedString> {
        let _ = cx;
        None
    }

    /// What the panel needs from the user right now, such as a permission to grant; `None`
    /// when nothing. The shell reads it again whenever the panel notifies. Only the side panel's
    /// toggle shows it so far, whether or not the panel is open.
    fn attention(&self, cx: &App) -> Option<SharedString> {
        let _ = cx;
        None
    }

    /// Who supplied the panel when it is not the application itself, such as "Plugin" for a
    /// user plugin's panel. The shell writes it as a chip beside the title, outside anything the
    /// panel draws, so a panel cannot pass for one of the application's; `None` for the
    /// application's own panels. `detail` is the chip's tooltip.
    fn provenance(&self, cx: &App) -> Option<Provenance> {
        let _ = cx;
        None
    }
}

/// The chip the shell writes beside the title of a panel the application did not supply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub chip: SharedString,
    pub detail: SharedString,
}

/// A [`Panel`] of any type, as the workspace holds it.
pub trait PanelHandle: 'static {
    fn panel_id(&self) -> EntityId;
    fn view(&self) -> AnyView;
    fn title(&self, cx: &App) -> SharedString;
    fn icon(&self, cx: &App) -> IconName;
    fn badge(&self, cx: &App) -> Option<SharedString>;
    fn provenance(&self, cx: &App) -> Option<Provenance>;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
}

impl<T: Panel> PanelHandle for Entity<T> {
    fn panel_id(&self) -> EntityId {
        self.entity_id()
    }

    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn title(&self, cx: &App) -> SharedString {
        self.read(cx).title(cx)
    }

    fn icon(&self, cx: &App) -> IconName {
        self.read(cx).icon(cx)
    }

    fn badge(&self, cx: &App) -> Option<SharedString> {
        self.read(cx).badge(cx)
    }

    fn provenance(&self, cx: &App) -> Option<Provenance> {
        self.read(cx).provenance(cx)
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }
}
