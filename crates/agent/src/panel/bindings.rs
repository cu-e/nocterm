//! Retained panel installation subscriptions; chat observations live with slots.
use gpui_kit::Subscription;
pub(super) struct PanelBindings {
    _subscriptions: Vec<Subscription>,
}
impl PanelBindings {
    pub(super) fn new(subscriptions: Vec<Subscription>) -> Self {
        Self {
            _subscriptions: subscriptions,
        }
    }
}
