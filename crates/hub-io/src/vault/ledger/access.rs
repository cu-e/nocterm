//! The keys and cache a ledger may use; captured operations never follow a new login.
use std::sync::Arc;

use nocterm_hubs_model::source_identity::LocalAccountSource;
use nocterm_vault::VaultError;

use crate::account::{OpenKeys, Slot, source::CapturedAccount};
use crate::cache::{AccountCache, VaultCopy};

pub(super) struct LedgerAccess {
    slot: Arc<Slot>,
    capture: Option<Arc<CapturedAccount>>,
}

impl LedgerAccess {
    #[cfg(test)]
    pub(super) fn live(slot: Arc<Slot>) -> Self {
        Self {
            slot,
            capture: None,
        }
    }

    pub(super) fn captured(slot: Arc<Slot>, capture: Arc<CapturedAccount>) -> Self {
        Self {
            slot,
            capture: Some(capture),
        }
    }

    pub(super) fn keys(&self) -> Result<Arc<OpenKeys>, VaultError> {
        match &self.capture {
            Some(capture) => capture.keys().map_err(|_| VaultError::Stale),
            None => self.slot.open().ok_or(VaultError::Locked),
        }
    }

    pub(super) fn read<R>(
        &self,
        read: impl FnOnce(Option<&AccountCache>) -> R,
    ) -> Result<R, VaultError> {
        match &self.capture {
            Some(capture) => {
                let keys = self.keys()?;
                let result = self.slot.read_cache_if(&keys, |cache| read(Some(cache)))?;
                capture.check_current().map_err(|_| VaultError::Stale)?;
                Ok(result)
            }
            None => Ok(self.slot.read_cache(read)),
        }
    }

    pub(super) fn publish_vault(&self, vault: VaultCopy) -> Result<(), VaultError> {
        match &self.capture {
            Some(capture) => {
                let keys = self.keys()?;
                self.slot
                    .update_cache_if_changed(&keys, |cache| cache.accept_vault(vault))?;
                capture.check_current().map_err(|_| VaultError::Stale)
            }
            None => {
                if let Err(error) = self
                    .slot
                    .update_cache_changed(|cache| cache.accept_vault(vault))
                {
                    tracing::warn!(%error, "the vault cache cannot be written");
                }
                Ok(())
            }
        }
    }
}
