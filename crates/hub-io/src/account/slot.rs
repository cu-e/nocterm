//! One native account Slot owns key lifecycle, original membership and private cache publication.
use super::{
    Notice, OpenKeys, Recovery, SlotReceipt, lifecycle::SlotAuthority, source::AccountSubject,
};
use crate::{
    cache::{AccountCache, CacheError, CacheFile},
    classify::credentials,
};
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
    cache: Option<AccountCache>,
    authority: SlotAuthority,
}

/// Everything this device knows of one hub's account keys.
pub(crate) struct Slot {
    hub: HubId,
    file: CacheFile,
    notice: Arc<Notice>,
    state: Mutex<SlotState>,
}

impl Slot {
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
                cache,
                authority: SlotAuthority::new(),
            }),
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

    pub(crate) fn install_if(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
        keys: OpenKeys,
        cache: AccountCache,
    ) -> Result<SlotReceipt, HubError> {
        let installed = {
            let mut state = self.state();
            state.authority.check(owner, receipt)?;
            if cache.account_id != owner.account() || keys.keyset().account_id() != owner.account()
            {
                return Err(HubError::Stale);
            }
            if let Err(error) = self.file.save(&cache) {
                tracing::warn!(%error, "the original account cache cannot be written");
            }
            state.cache = Some(cache);
            state.keys = KeyState::Open(Arc::new(keys.noticed_by(&self.notice)));
            state.authority.rotate()
        };
        self.notice.raise();
        Ok(installed)
    }

    pub(crate) fn cache_for(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
    ) -> Result<AccountCache, HubError> {
        let state = self.state();
        state.authority.check(owner, receipt)?;
        state.cache.clone().ok_or(HubError::NotFound)
    }

    pub(crate) fn change_cache_if(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
        change: impl FnOnce(&mut AccountCache),
    ) -> Result<(), HubError> {
        let mut state = self.state();
        state.authority.check(owner, receipt)?;
        let cache = state.cache.as_mut().ok_or(HubError::NotFound)?;
        change(cache);
        self.file.save(cache).map_err(credentials)
    }

    pub(crate) fn begin_recovery_issue(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
    ) -> Result<SlotReceipt, HubError> {
        let next = {
            let mut state = self.state();
            state.authority.check(owner, receipt)?;
            let cache = state.cache.as_mut().ok_or(HubError::NotFound)?;
            cache.recovery_unconfirmed = true;
            self.file.save(cache).map_err(credentials)?;
            state.authority.rotate()
        };
        self.notice.raise();
        Ok(next)
    }

    pub(crate) fn ack_recovery(
        &self,
        owner: &Arc<AccountSubject>,
        receipt: &SlotReceipt,
    ) -> Result<(), HubError> {
        {
            let mut state = self.state();
            state.authority.check(owner, receipt)?;
            let cache = state.cache.as_mut().ok_or(HubError::NotFound)?;
            cache.recovery_unconfirmed = false;
            self.file.save(cache).map_err(credentials)?;
            state.authority.rotate();
        }
        self.notice.raise();
        Ok(())
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
        read(self.state().cache.as_ref())
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
        Ok(read(cache))
    }

    /// The identity check and publication share the Slot mutex; a replacement
    /// account cannot receive a late cache produced by the original worker.
    pub(crate) fn update_cache_if(
        &self,
        expected: &Arc<OpenKeys>,
        change: impl FnOnce(&mut AccountCache),
    ) -> Result<(), nocterm_vault::VaultError> {
        self.update_cache_if_changed(expected, |cache| {
            change(cache);
            true
        })
    }

    pub(crate) fn update_cache_if_changed(
        &self,
        expected: &Arc<OpenKeys>,
        change: impl FnOnce(&mut AccountCache) -> bool,
    ) -> Result<(), nocterm_vault::VaultError> {
        let mut state = self.state();
        let current = matches!(&state.keys, KeyState::Open(keys) if Arc::ptr_eq(keys, expected));
        let cache = state
            .cache
            .as_mut()
            .filter(|cache| current && cache.account_id == expected.keyset().account_id())
            .ok_or(nocterm_vault::VaultError::Stale)?;
        if change(cache)
            && let Err(error) = self.file.save(cache)
        {
            tracing::warn!(%error, "the captured vault cache cannot be written");
        }
        Ok(())
    }

    /// Whether the recovery key of `account_id` was shown on this device and never confirmed.
    /// A cache of another account says nothing about this one.
    pub(crate) fn recovery_unconfirmed(&self, account_id: &str) -> bool {
        self.read_cache(|cache| {
            cache.is_some_and(|cache| cache.account_id == account_id && cache.recovery_unconfirmed)
        })
    }

    /// Changes the cached account and writes it through. Nothing happens without a cache.
    pub(crate) fn update_cache(
        &self,
        change: impl FnOnce(&mut AccountCache),
    ) -> Result<(), CacheError> {
        self.update_cache_changed(|cache| {
            change(cache);
            true
        })
    }

    pub(crate) fn update_cache_changed(
        &self,
        change: impl FnOnce(&mut AccountCache) -> bool,
    ) -> Result<(), CacheError> {
        let mut state = self.state();
        let Some(cache) = state.cache.as_mut() else {
            return Ok(());
        };
        if change(cache) {
            self.file.save(cache)?;
        }
        Ok(())
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
            state.cache = Some(cache);
            state.keys = KeyState::Open(Arc::new(keys.noticed_by(&self.notice)));
            state.authority.rotate();
        }
        self.notice.raise();
    }

    /// Opens the vault with `keys` from a cache that is already kept.
    #[cfg(test)]
    pub(crate) fn unlocked(&self, keys: OpenKeys) {
        {
            let mut state = self.state();
            state.keys = KeyState::Open(Arc::new(keys.noticed_by(&self.notice)));
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
