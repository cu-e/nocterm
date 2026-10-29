//! Actual captured key membership and one sticky native clock, no directory trust.
use super::*;
use crate::account::operations::{BoundOperations, OpenedSource};
use nocterm_hub_model::cloud::provider::SignedBootDescriptor;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

#[derive(Default)]
pub(in crate::cloud) struct BootClock {
    last: Option<Timestamp>,
    failed: bool,
}
impl BootClock {
    pub(in crate::cloud) fn observe(
        &mut self,
        now: Timestamp,
    ) -> Result<Timestamp, ProviderBootError> {
        if self.failed
            || now.as_millis() == 0
            || now.as_millis() > i64::MAX as u64
            || self.last.is_some_and(|old| now < old)
        {
            self.failed = true;
            return Err(ProviderBootError::Clock);
        }
        self.last = Some(now);
        Ok(now)
    }
}
pub(super) struct BootVerifier {
    bound: BoundOperations,
    binding: JournalCatalogBinding,
    pins: super::super::ChatMaterialPins,
    timer: Arc<dyn ToolTimer>,
    clock: Mutex<BootClock>,
    _allocation: AllocationLease,
}
impl BootVerifier {
    pub(super) fn new(
        bound: BoundOperations,
        binding: JournalCatalogBinding,
        pins: super::super::ChatMaterialPins,
        timer: Arc<dyn ToolTimer>,
        allocation: AllocationLease,
    ) -> Result<Self, ProviderBootError> {
        require_resident(&allocation)?;
        let value = Self {
            bound,
            binding,
            pins,
            timer,
            clock: Mutex::default(),
            _allocation: allocation,
        };
        value.check_current()?;
        Ok(value)
    }
    pub(super) fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub(super) fn source(&self) -> &Arc<OpenedSource> {
        self.bound.source()
    }
    pub(super) fn check_current(&self) -> Result<Timestamp, ProviderBootError> {
        self.source().check().map_err(account)?;
        let actual = self
            .source()
            .native()
            .with_current(|keys| {
                Ok(<[u8; 32]>::from(Sha256::digest(
                    keys.keyset().public_key().to_bytes(),
                )))
            })
            .map_err(account)?;
        let subject = self.source().native().subject().map_err(account)?;
        if actual != *self.binding.scope().account().key().as_bytes()
            || subject.account() != self.binding.scope().account().id().as_str()
        {
            return Err(ProviderBootError::Binding);
        }
        self.clock
            .lock()
            .map_err(|_| ProviderBootError::Clock)?
            .observe(self.timer.now())
            .map_err(|_| ProviderBootError::Clock)
    }
    pub(super) fn open(
        &self,
        bytes: &[u8],
        allocation: AllocationLease,
    ) -> Result<Allocated<VerifiedBootDescriptor>, ProviderBootError> {
        let result = (|| {
            require_work(&allocation)?;
            if bytes.len() > 580 {
                return Err(ProviderBootError::Capacity);
            }
            let now = self.check_current()?;
            let signed = SignedBootDescriptor::from_canonical_bytes(bytes).map_err(backend)?;
            let verified = signed
                .verify(
                    &self.pins.signing,
                    &super::super::metadata_crypto::namespace(&self.binding).map_err(backend)?,
                    super::super::metadata_crypto::runner(&self.binding),
                    i64::try_from(now.as_millis()).map_err(|_| ProviderBootError::Clock)?,
                )
                .map_err(|error| match error {
                    nocterm_hub_model::cloud::provider::ProviderLeaseError::Time => {
                        ProviderBootError::Expired
                    }
                    nocterm_hub_model::cloud::provider::ProviderLeaseError::Context => {
                        ProviderBootError::Binding
                    }
                    error => backend(error),
                })?;
            let now = self.check_current()?;
            verified
                .descriptor()
                .require_current(
                    i64::try_from(now.as_millis()).map_err(|_| ProviderBootError::Clock)?,
                )
                .map_err(|_| ProviderBootError::Expired)?;
            self.source()
                .publish(|| Allocated::new(verified, allocation.clone()))
                .map_err(account)
        })();
        result.map_err(|error| retained(error, allocation))
    }
}
