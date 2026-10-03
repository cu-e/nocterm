use gpui_kit::{AnyView, App, Entity, EntityId, FocusHandle, Focusable, Render, SharedString};
use nocterm_ui::IconName;

/// A view of the sidebar.
pub trait Panel: Render + Focusable {
    /// Shown above the panel and as the tooltip of its switcher icon.
    fn title(&self, cx: &App) -> SharedString;

    /// The panel's icon in the switcher strip at the foot of the sidebar.
    fn icon(&self, cx: &App) -> IconName;
}

/// A [`Panel`] of any type, as the workspace holds it.
pub trait PanelHandle: 'static {
    fn panel_id(&self) -> EntityId;
    fn view(&self) -> AnyView;
    fn title(&self, cx: &App) -> SharedString;
    fn icon(&self, cx: &App) -> IconName;
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

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }
}
