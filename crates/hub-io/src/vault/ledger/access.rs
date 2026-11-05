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
    pub(super) async fn operation(&self) -> Result<tokio::sync::OwnedSemaphorePermit, VaultError> {
        self.slot
            .cache_work()
            .operation()
            .await
            .map_err(|error| VaultError::Io(std::io::Error::other(error)))
    }

    pub(super) fn slot(&self) -> &Arc<Slot> {
        &self.slot
    }
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

    pub(super) async fn publish_vault(&self, vault: Arc<VaultCopy>) -> Result<(), VaultError> {
        let _turn = self.slot.cache_turn().await;
        let keys = self.keys()?;
        loop {
            let original = self.slot.cache_snapshot();
            let Some(mut next) = original.as_deref().cloned() else {
                return Err(VaultError::Stale);
            };
            if !next.accept_shared_vault(vault.clone()) {
                return Ok(());
            }
            let next = Arc::new(next);
            let draft = self
                .slot
                .stage_cache(original.clone(), next.as_ref().clone())
                .await;
            let result = draft.and_then(|draft| match &self.capture {
                Some(capture) => capture.with_current(|_| self.slot.commit_cache_if(&keys, draft)),
                None => self.slot.commit_cache_if(&keys, draft),
            });
            match result {
                Ok(Some(committed)) => {
                    self.slot.finish_cache(committed).await;
                    return Ok(());
                }
                Ok(None) => {}
                Err(error) if cache_write_failed(&error) => {
                    let accepted = match &self.capture {
                        Some(capture) => capture.with_current(|_| {
                            self.slot.accept_memory_cache_if(&keys, &original, next)
                        }),
                        None => self.slot.accept_memory_cache_if(&keys, &original, next),
                    }
                    .map_err(super::super::native_failure)?;
                    if accepted {
                        tracing::warn!(error = %nocterm_core::error::chain(&error), "the vault is current in memory; its offline cache could not be written");
                        return Ok(());
                    }
                }
                Err(error) => return Err(super::super::native_failure(error)),
            }
        }
    }
}

fn cache_write_failed(error: &nocterm_hubs_model::HubError) -> bool {
    matches!(error, nocterm_hubs_model::HubError::Credentials { cause }
        if matches!(cause.downcast_ref::<crate::cache::CacheError>(),
            Some(crate::cache::CacheError::Write(_) | crate::cache::CacheError::Prepare(_))))
}
