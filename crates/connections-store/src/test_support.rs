//! Controlled foreground source authority for consumers of captured probe leases.
use crate::{ProbeSourceAuthority, ProbeSourceIdentity, ProbeSourceLease};
use gpui::App;
use std::{cell::Cell, rc::Rc};
#[derive(Clone)]
pub struct ScriptedProbeSource {
    identity: ProbeSourceIdentity,
    current: Rc<Cell<bool>>,
}
impl ScriptedProbeSource {
    pub fn new(identity: ProbeSourceIdentity) -> Self {
        Self {
            identity,
            current: Rc::new(Cell::new(true)),
        }
    }
    pub fn lease(&self) -> ProbeSourceLease {
        ProbeSourceLease::new(self.identity, Rc::new(self.clone()))
    }
    pub fn set_current(&self, current: bool) {
        self.current.set(current);
    }
}
impl ProbeSourceAuthority for ScriptedProbeSource {
    fn is_current(&self, _: &App) -> bool {
        self.current.get()
    }
    fn same_session_alive(&self, _: &App) -> bool {
        self.current.get()
    }
}
