//! Retained UI bindings keep subscriptions alive without owning chat effects.
use gpui::{AnyWindowHandle, Subscription};
pub(super) struct ThreadBindings {
    window: Option<AnyWindowHandle>,
    _release: Subscription,
    _workspace_authority: Option<Subscription>,
}
impl ThreadBindings {
    pub(super) fn new(release: Subscription, workspace_authority: Option<Subscription>) -> Self {
        Self {
            window: None,
            _release: release,
            _workspace_authority: workspace_authority,
        }
    }
    pub(super) fn bind_window(&mut self, window: AnyWindowHandle) {
        self.window = Some(window);
    }
    pub(super) fn window(&self) -> Option<AnyWindowHandle> {
        self.window
    }
}
