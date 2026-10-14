//! Foreground original-source authority; no view/App value crosses native work.
use gpui::App;
use std::{cell::Cell, rc::Rc};
pub trait ProbeSourceAuthority {
    fn is_current(&self, cx: &App) -> bool;
    /// Original Item/session lifetime, independent of profile association or country policy.
    fn same_session_alive(&self, cx: &App) -> bool;
}
/// Consumer-neutral identity of the original GPUI Item and captured session epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeSourceIdentity {
    item: gpui::EntityId,
    generation: u64,
}
impl ProbeSourceIdentity {
    pub fn new(item: gpui::EntityId, generation: u64) -> Self {
        Self { item, generation }
    }
    pub(super) fn item(self) -> gpui::EntityId {
        self.item
    }
}
#[derive(Clone)]
pub struct ProbeSourceLease {
    identity: ProbeSourceIdentity,
    revoked: Rc<Cell<bool>>,
    source: Rc<dyn ProbeSourceAuthority>,
}
impl ProbeSourceLease {
    pub fn new(identity: ProbeSourceIdentity, source: Rc<dyn ProbeSourceAuthority>) -> Self {
        Self {
            identity,
            revoked: Rc::new(Cell::new(false)),
            source,
        }
    }
    pub(super) fn identity(&self) -> ProbeSourceIdentity {
        self.identity
    }
    pub fn revoke(&self) {
        self.revoked.set(true);
    }
    pub(super) fn same_session_alive(&self, cx: &App) -> bool {
        self.source.same_session_alive(cx)
    }
    pub fn is_current(&self, cx: &App) -> bool {
        !self.revoked.get() && self.source.is_current(cx)
    }
}
