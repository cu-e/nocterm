//! One native account Slot owns key lifecycle, original membership and private cache publication.
//! Immutable cache generations separate content contention from lifecycle receipts.
//! A serialized async writer stages on shared bounded workers; admission retains
//! only the final generation check, namespace replacement and memory transition.
use super::cache_publication::CacheDraft;
use super::{
    Notice, OpenKeys, Recovery, SlotReceipt, lifecycle::SlotAuthority, source::AccountSubject,
};
use crate::{
    cache::{AccountCache, CacheFile},
    classify::credentials,
};
use nocterm_core::fs::CommittedPublication;
use nocterm_hubs_model::{HubError, HubId};
use nocterm_vault::VaultStatus;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

pub(crate) struct SlotEnd {
    receipt: SlotReceipt,
    result: Result<(), HubError>,
}
impl SlotEnd {
    pub(crate) fn into_parts(self) -> (SlotReceipt, Result<(), HubError>) {
        (self.receipt, self.result)
    }
}

enum KeyState {
    Locked,
    Open(Arc<OpenKeys>),
    Recovering(Arc<Recovery>),
}

struct SlotState {
    keys: KeyState,
    cache: Option<Arc<AccountCache>>,
    authority: SlotAuthority,
    pending_recovery: Option<SlotReceipt>,
}

/// Everything this device knows of one hub's account keys.
pub(crate) struct Slot {
    hub: HubId,
    file: CacheFile,
    notice: Arc<Notice>,
    state: Mutex<SlotState>,
    cache_write: tokio::sync::Mutex<()>,
    refresh: super::refresh::Refresh,
}

impl Slot {
    pub(crate) fn pending_recovery_for(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
    ) -> Result<bool, HubError> {
        let state = self.state();
        state.authority.check(owner, receipt)?;
        Ok(state
            .cache
            .as_ref()
            .is_some_and(|cache| cache.account_id == owner.account() && cache.recovery_unconfirmed)
            || state
                .pending_recovery
                .as_ref()
                .is_some_and(|pending| state.authority.check_observed(pending).is_ok()))
    }

    /// A remote keyset was created, but its first durable local installation
    /// failed. Keep only this bounded fact for the original current generation.
    pub(crate) fn remember_pending_recovery(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
    ) -> Result<SlotReceipt, HubError> {
        let next = {
            let mut state = self.state();
            state.authority.check(owner, receipt)?;
            let next = state.authority.rotate();
            state.pending_recovery = Some(next.clone());
            next
        };
        self.notice.raise();
        Ok(next)
    }
    pub(crate) async fn refresh_turn(
        &self,
        keys: &Arc<OpenKeys>,
    ) -> super::refresh::RefreshTurn<'_> {
        self.refresh.enter(keys).await
    }

    pub(crate) fn cache_work(&self) -> &Arc<crate::cache::CacheWork> {
        self.file.work()
    }
    pub(crate) async fn cache_turn(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.cache_write.lock().await
    }

    pub(crate) fn cache_snapshot(&self) -> Option<Arc<AccountCache>> {
        self.state().cache.clone()
    }

    pub(crate) async fn stage_cache(
        &self,
        original: Option<Arc<AccountCache>>,
        next: AccountCache,
    ) -> Result<CacheDraft, HubError> {
        let file = self.file.clone();
        self.file
            .work()
            .run(move || {
                let publication = file.stage(&next)?;
                Ok(CacheDraft::new(original, next, publication))
            })
            .await
            .map_err(credentials)
    }

    pub(crate) async fn finish_cache(&self, committed: CommittedPublication) {
        if let Err(error) = self
            .file
            .work()
            .run(move || {
                committed.sync_parent();
                Ok(())
            })
            .await
        {
            tracing::warn!(%error, "the committed account cache parent cannot be synced");
        }
    }

    pub(crate) fn commit_cache_if(
        &self,
        expected: &Arc<OpenKeys>,
        draft: CacheDraft,
    ) -> Result<Option<CommittedPublication>, HubError> {
        let mut state = self.state();
        if !matches!(&state.keys, KeyState::Open(keys) if Arc::ptr_eq(keys, expected))
            || draft.account() != expected.keyset().account_id()
        {
            return Err(HubError::Stale);
        }
        if !draft.matches(&state.cache) {
            return Ok(None);
        }
        let (next, committed) = draft.commit().map_err(credentials)?;
        state.cache = Some(next);
        Ok(Some(committed))
    }

    /// Ordinary encrypted refresh remains usable when its offline copy cannot
    /// be written. Security facts use the durable commit methods exclusively.
    pub(crate) fn accept_memory_cache_if(
        &self,
        expected: &Arc<OpenKeys>,
        original: &Option<Arc<AccountCache>>,
        next: Arc<AccountCache>,
    ) -> Result<bool, HubError> {
        let mut state = self.state();
        if !matches!(&state.keys, KeyState::Open(keys) if Arc::ptr_eq(keys, expected))
            || next.account_id != expected.keyset().account_id()
        {
            return Err(HubError::Stale);
        }
        if !super::cache_publication::same_generation(original, &state.cache) {
            return Ok(false);
        }
        state.cache = Some(next);
        Ok(true)
    }

    pub(crate) fn commit_member_cache(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
        draft: CacheDraft,
        rotate: bool,
    ) -> Result<Option<(SlotReceipt, CommittedPublication)>, HubError> {
        let mut state = self.state();
        state.authority.check(owner, receipt)?;
        if draft.account() != owner.account() {
            return Err(HubError::Stale);
        }
        if !draft.matches(&state.cache) {
            return Ok(None);
        }
        let (next, committed) = draft.commit().map_err(credentials)?;
        state.cache = Some(next);
        state.pending_recovery = None;
        let receipt = if rotate {
            state.authority.rotate()
        } else {
            receipt.clone()
        };
        Ok(Some((receipt, committed)))
    }

    pub(crate) fn commit_install(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
        keys: Arc<OpenKeys>,
        draft: CacheDraft,
    ) -> Result<Option<(SlotReceipt, CommittedPublication)>, HubError> {
        let mut state = self.state();
        state.authority.check(owner, receipt)?;
        if draft.account() != owner.account() || keys.keyset().account_id() != owner.account() {
            return Err(HubError::Stale);
        }
        if !draft.matches(&state.cache) {
            return Ok(None);
        }
        let (next, committed) = draft.commit().map_err(credentials)?;
        state.cache = Some(next);
        state.keys = KeyState::Open(keys);
        state.pending_recovery = None;
        Ok(Some((state.authority.rotate(), committed)))
    }

    pub(super) fn new(hub: HubId, file: CacheFile, notice: Arc<Notice>) -> Self {
        let cache = match file.load() {
            Ok(cache) => cache,
            Err(error) => {
                tracing::warn!(%error, hub = %hub, "the account cache is not usable; it is read again from the hub");
                None
            }
        };
        Self {
            hub,
            file,
            notice,
            state: Mutex::new(SlotState {
                keys: KeyState::Locked,
                cache: cache.map(Arc::new),
                authority: SlotAuthority::new(),
                pending_recovery: None,
            }),
            cache_write: tokio::sync::Mutex::new(()),
            refresh: super::refresh::Refresh::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn hub(&self) -> &HubId {
        &self.hub
    }

    pub(crate) fn notice(&self) -> &Arc<Notice> {
        &self.notice
    }

    /// Called under Pool's clients mutex before a new membership is visible.
    pub(crate) fn begin_membership(&self, owner: &Arc<AccountSubject>) {
        {
            let mut state = self.state();
            state.keys = KeyState::Locked;
            state.authority.begin(owner);
            state.pending_recovery = None;
            if state
                .cache
                .as_ref()
                .is_some_and(|cache| cache.account_id != owner.account())
            {
                state.cache = None;
                if let Err(error) = self.file.erase() {
                    tracing::warn!(%error, "the former account cache cannot be erased");
                }
            }
        }
        self.notice.raise();
    }

    pub(crate) fn receipt(&self, owner: &Arc<AccountSubject>) -> Result<SlotReceipt, HubError> {
        self.state().authority.receipt(owner)
    }

    pub(crate) fn check_receipt(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
    ) -> Result<(), HubError> {
        self.state().authority.check(owner, receipt)
    }

    pub(crate) fn observed(&self) -> SlotReceipt {
        self.state().authority.observed()
    }
    pub(crate) fn check_observed(&self, receipt: &SlotReceipt) -> Result<(), HubError> {
        self.state().authority.check_observed(receipt)
    }
    pub(crate) fn belongs_to(&self, owner: &Arc<AccountSubject>) -> Result<(), HubError> {
        self.state().authority.check_owner(owner)
    }

    pub(crate) fn close_for(&self, owner: &Arc<AccountSubject>) -> Result<SlotReceipt, HubError> {
        let next = {
            let mut state = self.state();
            state.authority.check_owner(owner)?;
            state.keys = KeyState::Locked;
            state.authority.rotate()
        };
        self.notice.raise();
        Ok(next)
    }

    pub(crate) fn end_observed(
        &self,
        receipt: &SlotReceipt,
        erase: bool,
    ) -> Result<SlotEnd, HubError> {
        let ended = {
            let mut state = self.state();
            state.authority.check_observed(receipt)?;
            state.keys = KeyState::Locked;
            if erase {
                state.cache = None;
            }
            let receipt = state.authority.rotate();
            let result = if erase {
                self.file.erase().map_err(credentials)
            } else {
                Ok(())
            };
            SlotEnd { receipt, result }
        };
        self.notice.raise();
        Ok(ended)
    }

    pub(crate) fn open_for(&self, owner: &Arc<AccountSubject>) -> Result<Arc<OpenKeys>, HubError> {
        let state = self.state();
        state.authority.check_owner(owner)?;
        match &state.keys {
            KeyState::Open(keys) if keys.keyset().account_id() == owner.account() => {
                Ok(keys.clone())
            }
            _ => Err(HubError::KeysLocked),
        }
    }

    pub(crate) fn cache_for(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
    ) -> Result<AccountCache, HubError> {
        let state = self.state();
        state.authority.check(owner, receipt)?;
        state.cache.as_deref().cloned().ok_or(HubError::NotFound)
    }

    pub(super) fn recovering_if(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
        recovery: Recovery,
    ) -> Result<SlotReceipt, HubError> {
        let next = {
            let mut state = self.state();
            state.authority.check(owner, receipt)?;
            if recovery.account_id != owner.account() {
                return Err(HubError::Stale);
            }
            state.keys = KeyState::Recovering(Arc::new(recovery));
            state.authority.rotate()
        };
        self.notice.raise();
        Ok(next)
    }

    pub(crate) fn lock_keys_if(&self, expected: &Arc<OpenKeys>) -> bool {
        {
            let mut state = self.state();
            if !matches!(&state.keys, KeyState::Open(keys) if Arc::ptr_eq(keys, expected)) {
                return false;
            }
            state.keys = KeyState::Locked;
            state.authority.rotate();
        }
        self.notice.raise();
        true
    }

    pub(crate) fn while_keys_current<T>(
        &self,
        expected: &Arc<OpenKeys>,
        publish: impl FnOnce() -> T,
    ) -> Result<T, HubError> {
        let state = self.state();
        if !matches!(&state.keys, KeyState::Open(keys) if Arc::ptr_eq(keys, expected)) {
            return Err(HubError::Stale);
        }
        Ok(publish())
    }

    pub(crate) fn erase_for(&self, owner: &Arc<AccountSubject>) -> Result<(), HubError> {
        let result = {
            let mut state = self.state();
            state.authority.check_owner(owner)?;
            state.keys = KeyState::Locked;
            state.cache = None;
            state.authority.rotate();
            self.file.erase().map_err(credentials)
        };
        self.notice.raise();
        result
    }

    /// The keys, while the vault is unlocked.
    pub(crate) fn open(&self) -> Option<Arc<OpenKeys>> {
        match &self.state().keys {
            KeyState::Open(keys) => Some(Arc::clone(keys)),
            _ => None,
        }
    }

    pub(crate) fn status(&self) -> VaultStatus {
        let state = self.state();
        match (&state.keys, &state.cache) {
            (KeyState::Open(_), _) => VaultStatus::Unlocked,
            (_, Some(_)) => VaultStatus::Locked,
            _ => VaultStatus::Missing,
        }
    }

    /// Reads the cached account, if the hub has one on this device.
    pub(crate) fn read_cache<R>(&self, read: impl FnOnce(Option<&AccountCache>) -> R) -> R {
        let snapshot = self.state().cache.clone();
        read(snapshot.as_deref())
    }

    /// Uses only the original open account, under the same mutex as cache access.
    pub(crate) fn read_cache_if<R>(
        &self,
        expected: &Arc<OpenKeys>,
        read: impl FnOnce(&AccountCache) -> R,
    ) -> Result<R, nocterm_vault::VaultError> {
        let state = self.state();
        let current = matches!(&state.keys, KeyState::Open(keys) if Arc::ptr_eq(keys, expected));
        let cache = state
            .cache
            .as_ref()
            .filter(|cache| current && cache.account_id == expected.keyset().account_id())
            .ok_or(nocterm_vault::VaultError::Stale)?;
        let cache = cache.clone();
        drop(state);
        Ok(read(&cache))
    }

    /// Whether the recovery key of `account_id` was shown on this device and never confirmed.
    /// A cache of another account says nothing about this one.
    pub(crate) fn recovery_unconfirmed(&self, account_id: &str) -> bool {
        self.read_cache(|cache| {
            cache.is_some_and(|cache| cache.account_id == account_id && cache.recovery_unconfirmed)
        })
    }

    /// Opens the vault with `keys`, remembering `cache` on disk. A cache that cannot be
    /// written is logged: the account still works, but cannot unlock without the hub.
    #[cfg(test)]
    pub(crate) fn install(&self, keys: OpenKeys, cache: AccountCache) {
        {
            let mut state = self.state();
            if let Err(error) = self.file.save(&cache) {
                tracing::warn!(%error, hub = %self.hub, "the account cache cannot be written");
            }
            state.cache = Some(Arc::new(cache));
            state.keys = KeyState::Open(Arc::new(keys));
            state.authority.rotate();
        }
        self.notice.raise();
    }

    /// Opens the vault with `keys` from a cache that is already kept.
    #[cfg(test)]
    pub(crate) fn unlocked(&self, keys: OpenKeys) {
        {
            let mut state = self.state();
            state.keys = KeyState::Open(Arc::new(keys));
            state.authority.rotate();
        }
        self.notice.raise();
    }

    pub(super) fn recovery(&self) -> Option<Arc<Recovery>> {
        match &self.state().keys {
            KeyState::Recovering(recovery) => Some(Arc::clone(recovery)),
            _ => None,
        }
    }

    /// Drops the keys: the vault is locked, the cache stays.
    pub(crate) fn lock(&self) {
        let was_unlocked = {
            let mut state = self.state();
            let was = !matches!(state.keys, KeyState::Locked);
            state.keys = KeyState::Locked;
            state.authority.rotate();
            was
        };
        if was_unlocked {
            self.notice.raise();
        }
    }

    /// A captured revoked operation may erase only its original open account.
    pub(crate) fn erase_if(&self, expected: &Arc<OpenKeys>) -> Result<(), HubError> {
        let mut state = self.state();
        if !matches!(&state.keys, KeyState::Open(keys) if Arc::ptr_eq(keys, expected))
            || !state
                .cache
                .as_ref()
                .is_some_and(|cache| cache.account_id == expected.keyset().account_id())
        {
            return Ok(());
        }
        state.keys = KeyState::Locked;
        state.cache = None;
        state.authority.rotate();
        let result = self.file.erase().map_err(credentials);
        drop(state);
        self.notice.raise();
        result
    }

    /// Drops the keys and erases the cache: the account is signed out of this device.
    pub(crate) fn erase(&self) -> Result<(), HubError> {
        {
            let mut state = self.state();
            state.keys = KeyState::Locked;
            state.cache = None;
            state.authority.rotate();
        }
        self.notice.raise();
        self.file.erase().map_err(credentials)
    }
}
