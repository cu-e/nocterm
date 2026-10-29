//! Synchronous staging only: the catalog publishes after the source returns current.
use super::*;
use crate::MAX_PROVIDER_LEASE;
use nocterm_cloud_model::coordinator::ProviderCredentialReceiver;
use nocterm_core::allocation::AllocationLease;
use std::time::Duration;
use zeroize::Zeroizing;

pub(super) struct Candidate {
    binding: ProviderBinding,
    expires: Timestamp,
    source: Arc<dyn ProviderCredentialSource>,
    timer: Arc<dyn ToolTimer>,
    lease: Option<ProviderLease>,
}
impl Candidate {
    pub(super) fn new(
        binding: ProviderBinding,
        expires: Timestamp,
        source: Arc<dyn ProviderCredentialSource>,
        timer: Arc<dyn ToolTimer>,
    ) -> Self {
        Self {
            binding,
            expires,
            source,
            timer,
            lease: None,
        }
    }
    pub(super) fn take(&mut self) -> Result<ProviderLease, CoordinatorError> {
        self.lease
            .take()
            .ok_or(CoordinatorError::NeedsProviderAccess)
    }
}
impl ProviderCredentialReceiver for Candidate {
    fn install(
        &mut self,
        secret: &[u8],
        allocation: AllocationLease,
    ) -> Result<(), CoordinatorError> {
        if self.lease.is_some()
            || secret.is_empty()
            || secret.len() > 16 * 1024
            || allocation.bytes() < secret.len()
            || !secret.iter().all(u8::is_ascii_graphic)
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        self.source.check_current()?;
        let remaining = self
            .expires
            .as_millis()
            .checked_sub(self.timer.now().as_millis())
            .ok_or(CoordinatorError::Expired)?;
        let lifetime = Duration::from_millis(remaining).min(MAX_PROVIDER_LEASE);
        let text = std::str::from_utf8(secret).map_err(|_| CoordinatorError::InvalidCommand)?;
        let lease = ProviderLease::with_residency(
            self.binding.clone(),
            Zeroizing::new(text.to_owned()),
            lifetime,
            allocation,
            self.source.clone(),
            self.timer.clone(),
            self.expires,
        )?;
        self.lease = Some(lease);
        Ok(())
    }
}
impl Drop for Candidate {
    fn drop(&mut self) {
        if let Some(lease) = &self.lease {
            lease.revoke();
        }
    }
}
