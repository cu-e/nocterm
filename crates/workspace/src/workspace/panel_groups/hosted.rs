//! One panel hosted by a group: its erased handle, the presentation it was last seen with
//! and the lifetime of the observer that keeps it current.

use gpui_kit::{AnyView, App, EntityId, FocusHandle, SharedString, Subscription};
use nocterm_plugin_model::PanelChrome;
use nocterm_ui::IconName;

use crate::{Panel, PanelHandle, right_panel::RightPanelHandle};

/// What the chrome draws for a panel outside its view: title, icon and badge.
pub(in crate::workspace) type Presentation = (SharedString, IconName, Option<SharedString>);

enum Handle {
    Plain(Box<dyn PanelHandle>),
    Expandable(Box<dyn RightPanelHandle>),
}

pub(in crate::workspace) struct HostedPanel {
    handle: Handle,
    presentation: Presentation,
    chrome: PanelChrome,
    /// The panel asks for the user; its toggle shows this whether or not it is open.
    attention: bool,
    _subscription: Subscription,
}

impl HostedPanel {
    /// A panel that the workspace titles and that cannot expand.
    pub(in crate::workspace) fn plain(
        handle: Box<dyn PanelHandle>,
        presentation: Presentation,
        subscription: Subscription,
    ) -> Self {
        Self::new(
            Handle::Plain(handle),
            PanelChrome::Titled,
            presentation,
            subscription,
        )
    }
    /// A panel that draws its own header and can expand over the content.
    pub(in crate::workspace) fn expandable(
        handle: Box<dyn RightPanelHandle>,
        presentation: Presentation,
        subscription: Subscription,
    ) -> Self {
        Self::new(
            Handle::Expandable(handle),
            PanelChrome::OwnHeader,
            presentation,
            subscription,
        )
    }
    fn new(
        handle: Handle,
        chrome: PanelChrome,
        presentation: Presentation,
        subscription: Subscription,
    ) -> Self {
        Self {
            handle,
            chrome,
            presentation,
            attention: false,
            _subscription: subscription,
        }
    }
    /// Starts with the attention the panel asks for.
    pub(in crate::workspace) fn with_attention(mut self, attention: bool) -> Self {
        self.attention = attention;
        self
    }
    /// Frames the panel as its contribution says instead of by how it is hosted.
    pub(in crate::workspace) fn with_chrome(mut self, chrome: PanelChrome) -> Self {
        self.chrome = chrome;
        self
    }
    fn handle(&self) -> &dyn PanelHandle {
        match &self.handle {
            Handle::Plain(handle) => handle.as_ref(),
            Handle::Expandable(handle) => handle.as_ref(),
        }
    }
    pub(in crate::workspace) fn panel_id(&self) -> EntityId {
        self.handle().panel_id()
    }
    pub(in crate::workspace) fn title(&self, _: &App) -> SharedString {
        self.presentation.0.clone()
    }
    pub(in crate::workspace) fn icon(&self, _: &App) -> IconName {
        self.presentation.1
    }
    pub(in crate::workspace) fn badge(&self, _: &App) -> Option<SharedString> {
        self.presentation.2.clone()
    }
    pub(in crate::workspace) fn view(&self) -> AnyView {
        self.handle().view()
    }
    pub(in crate::workspace) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.handle().focus_handle(cx)
    }
    pub(in crate::workspace) fn chrome(&self) -> PanelChrome {
        self.chrome
    }
    pub(in crate::workspace) fn is_expandable(&self) -> bool {
        matches!(self.handle, Handle::Expandable(_))
    }
    pub(in crate::workspace) fn is<T: Panel>(&self) -> bool {
        self.handle().view().downcast::<T>().is_ok()
    }
    pub(in crate::workspace) fn attention(&self) -> bool {
        self.attention
    }
    /// Whether the attention flag changed.
    pub(in crate::workspace) fn set_attention(&mut self, attention: bool) -> bool {
        if self.attention == attention {
            return false;
        }
        self.attention = attention;
        true
    }
    /// Whether the presentation changed.
    pub(in crate::workspace) fn presentation_changed(
        &mut self,
        presentation: Presentation,
    ) -> bool {
        if self.presentation == presentation {
            return false;
        }
        self.presentation = presentation;
        true
    }
    /// Tells an expandable panel that it fills the body; others ignore it.
    pub(super) fn set_expanded(&self, expanded: bool, cx: &mut App) {
        if let Handle::Expandable(handle) = &self.handle {
            handle.set_maximized(expanded, cx);
        }
    }
    /// Tells an expandable panel which side of the content it sits on; others ignore it.
    pub(super) fn set_docked_left(&self, left: bool, cx: &mut App) {
        if let Handle::Expandable(handle) = &self.handle {
            handle.set_docked_left(left, cx);
        }
    }
}
