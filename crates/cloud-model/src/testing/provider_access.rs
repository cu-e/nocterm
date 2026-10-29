//! Explicit test credentials and deterministic post-delivery suspension; no crypto authority.
use crate::{coordinator::*, identity::CommandId, journal::JournalCatalogBinding};
use nocterm_core::allocation::{Allocated, AllocationLease};
use nocterm_session::testing::Gate;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

struct ScriptedCredential {
    command: CommandAcknowledgement,
    offer: ProviderOffer,
    secret: Allocated<Vec<u8>>,
}
pub struct ScriptedProviderCredentials {
    binding: JournalCatalogBinding,
    current: AtomicBool,
    staged: Mutex<BTreeMap<CommandId, ScriptedCredential>>,
    pause: Mutex<Option<(Gate, Gate)>>,
}
impl ScriptedProviderCredentials {
    pub fn new(binding: JournalCatalogBinding) -> Self {
        Self {
            binding,
            current: AtomicBool::new(true),
            staged: Mutex::new(BTreeMap::new()),
            pause: Mutex::new(None),
        }
    }
    pub fn stage(
        &self,
        command: CommandAcknowledgement,
        offer: ProviderOffer,
        secret: Vec<u8>,
        allocation: AllocationLease,
    ) -> Result<(), CoordinatorError> {
        if !self.binding.contains(command.stream()) || secret.len() > allocation.bytes() {
            return Err(CoordinatorError::InvalidCommand);
        }
        let mut staged = self.staged.lock().unwrap_or_else(PoisonError::into_inner);
        if staged.len() >= 128 || staged.contains_key(&command.acceptance().id()) {
            return Err(CoordinatorError::Overloaded);
        }
        staged.insert(
            command.acceptance().id(),
            ScriptedCredential {
                command,
                offer,
                secret: Allocated::new(secret, allocation),
            },
        );
        Ok(())
    }
    /// The first gate opens when a candidate exists; releasing the second lets
    /// consume return. No fake lock is held while the receiver or waiter runs.
    pub fn pause_next_delivery(&self) -> (Gate, Gate) {
        let gates = (Gate::pending(), Gate::pending());
        *self.pause.lock().unwrap_or_else(PoisonError::into_inner) = Some(gates.clone());
        gates
    }
    pub fn revoke(&self) {
        self.current.store(false, Ordering::Release);
    }
}
impl ProviderCredentialSource for ScriptedProviderCredentials {
    fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    fn check_current(&self) -> Result<(), CoordinatorError> {
        if self.current.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(CoordinatorError::StaleSource)
        }
    }
    fn consume(
        &self,
        command: &CommandAcknowledgement,
        offer: &ProviderOffer,
        receiver: &mut dyn ProviderCredentialReceiver,
    ) -> Result<(), CoordinatorError> {
        self.check_current()?;
        let original = self
            .staged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&command.acceptance().id())
            .ok_or(CoordinatorError::NeedsProviderAccess)?;
        if original.command.stream() != command.stream()
            || original.command.acceptance() != command.acceptance()
            || original.offer.reference() != offer.reference()
            || original.offer.provider() != offer.provider()
            || original.offer.models() != offer.models()
            || original.offer.boot() != offer.boot()
            || original.offer.context() != offer.context()
            || original.offer.expires() != offer.expires()
        {
            return Err(CoordinatorError::Conflict);
        }
        receiver.install(
            original.secret.value(),
            original.secret.allocation().clone(),
        )?;
        let pause = self
            .pause
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some((entered, release)) = pause {
            entered.release();
            futures::executor::block_on(release.wait());
        }
        self.check_current()
    }
}

#[derive(Default)]
pub struct RecordingProviderCredentialReceiver {
    received: Option<Allocated<Vec<u8>>>,
}
impl RecordingProviderCredentialReceiver {
    pub fn received(&self) -> Option<&Allocated<Vec<u8>>> {
        self.received.as_ref()
    }
}
impl ProviderCredentialReceiver for RecordingProviderCredentialReceiver {
    fn install(
        &mut self,
        secret: &[u8],
        allocation: AllocationLease,
    ) -> Result<(), CoordinatorError> {
        if self.received.is_some() || allocation.bytes() < secret.len() {
            return Err(CoordinatorError::InvalidCommand);
        }
        self.received = Some(Allocated::new(secret.to_vec(), allocation));
        Ok(())
    }
}
