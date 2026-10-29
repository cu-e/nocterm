//! An unaccepted candidate remains owned even when its waiter disappears.
//! Connected reattachment atomically transfers physical cleanup to its supervisor.
use super::{HeadlessError, SessionRegistry, physical::PhysicalTerminal};
use std::sync::{Arc, Mutex, PoisonError};

pub(super) struct PendingOpen {
    registry: Arc<Mutex<SessionRegistry>>,
    terminal: String,
    original: Arc<Mutex<PhysicalTerminal>>,
    connected: bool,
}
impl PendingOpen {
    pub(super) fn new(
        registry: Arc<Mutex<SessionRegistry>>,
        terminal: String,
        original: Arc<Mutex<PhysicalTerminal>>,
    ) -> Self {
        Self {
            registry,
            terminal,
            original,
            connected: false,
        }
    }
    pub(super) fn connected(mut self) -> Result<(), HeadlessError> {
        self.original
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .publish()?;
        self.connected = true;
        Ok(())
    }
}
impl Drop for PendingOpen {
    fn drop(&mut self) {
        if !self.connected {
            self.registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove_if_original(&self.terminal, &self.original);
        }
    }
}
