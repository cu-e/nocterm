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
    pub(super) fn publish_initial(self) -> Result<(), HeadlessError> {
        self.publish_initial_owned()?.accept();
        Ok(())
    }
    pub(super) fn publish_initial_owned(mut self) -> Result<InitialPublication, HeadlessError> {
        let owner = Arc::new(());
        self.registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .publish_initial(&self.terminal, &self.original, owner.clone())?;
        self.connected = true;
        Ok(InitialPublication {
            registry: self.registry.clone(),
            terminal: self.terminal.clone(),
            original: self.original.clone(),
            owner,
            accepted: false,
        })
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

/// A published physical candidate is still cancellable until its original run
/// accepts it. Matching the actual pointer cannot close a later acquisition.
pub(super) struct InitialPublication {
    registry: Arc<Mutex<SessionRegistry>>,
    terminal: String,
    original: Arc<Mutex<PhysicalTerminal>>,
    owner: Arc<()>,
    accepted: bool,
}
impl InitialPublication {
    pub(super) fn accept(mut self) {
        self.original
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .accept_initial(&self.owner);
        self.accepted = true;
    }
}
impl Drop for InitialPublication {
    fn drop(&mut self) {
        if !self.accepted {
            self.registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retire_initial_if_original(&self.terminal, &self.original, &self.owner);
        }
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
