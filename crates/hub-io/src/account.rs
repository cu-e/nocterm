//! The open keys of the account, per hub, kept in memory only while the vault is unlocked.
//!
//! A hub's [`Slot`] holds the keyset opened with the vault password (never the password or
//! the unlock key beyond opening it), the collection keys opened from it, and a copy of the
//! encrypted cache file. Locking drops the keys, which wipes them. One notice is raised for
//! every change so a vault can report its status without polling.

mod flow;
#[cfg(test)]
pub(crate) mod testing;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nocterm_hub_client::Keyset;
use nocterm_hubs_model::{HubError, HubId};
use nocterm_keys::{CollectionKey, OpenError, UnlockKdf, UnlockKey};
use nocterm_vault::VaultStatus;
use tokio::sync::watch;

use crate::cache::{AccountCache, CacheError, CacheFile, UnlockMethod};
use crate::classify::{credentials, remote};

pub(crate) use flow::KeyFlow;

/// Raises a counter whenever keys are opened, dropped or replaced.
pub(crate) struct Notice(watch::Sender<u64>);

impl Notice {
    fn new() -> Self {
        Self(watch::channel(0).0)
    }

    fn raise(&self) {
        self.0.send_modify(|count| *count += 1);
    }

    pub(crate) fn count(&self) -> u64 {
        *self.0.borrow()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}

/// The opened keyset and the collection keys derived from it.
pub(crate) struct OpenKeys {
    keyset: Keyset,
    collections: Mutex<HashMap<String, Arc<CollectionKey>>>,
}

impl OpenKeys {
    pub(crate) fn new(keyset: Keyset) -> Self {
        Self {
            keyset,
            collections: Mutex::default(),
        }
    }

    fn collections(&self) -> MutexGuard<'_, HashMap<String, Arc<CollectionKey>>> {
        self.collections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The key of a collection, opened from the envelope the hub holds for this account.
    /// An envelope that does not open is [`OpenError`]: the hub altered it.
    pub(crate) fn collection(
        &self,
        collection_id: &str,
        envelope: &[u8],
    ) -> Result<Arc<CollectionKey>, OpenError> {
        if let Some(key) = self.collections().get(collection_id) {
            return Ok(Arc::clone(key));
        }
        let key = Arc::new(self.keyset.open_collection_key(collection_id, envelope)?);
        self.collections()
            .insert(collection_id.to_owned(), Arc::clone(&key));
        Ok(key)
    }

    /// A new collection key of its own, with the envelope the hub stores for this account.
    pub(crate) fn new_collection(
        &self,
        collection_id: &str,
    ) -> Result<(Arc<CollectionKey>, Vec<u8>), HubError> {
        let key = CollectionKey::generate().map_err(remote)?;
        let envelope = self
            .keyset
            .seal_collection_key(&key, collection_id)
            .map_err(remote)?;
        let key = Arc::new(key);
        self.collections()
            .insert(collection_id.to_owned(), Arc::clone(&key));
        Ok((key, envelope))
    }
}

/// A keyset the typed password does not open, kept until a recovery key opens it.
pub(crate) struct Recovery {
    account_id: String,
    hub_uid: String,
    unlock: UnlockKey,
    kdf: UnlockKdf,
    method: UnlockMethod,
}

enum KeyState {
    Locked,
    Open(Arc<OpenKeys>),
    Recovering(Arc<Recovery>),
}

struct SlotState {
    keys: KeyState,
    cache: Option<AccountCache>,
}

/// Everything this device knows of one hub's account keys.
pub(crate) struct Slot {
    hub: HubId,
    file: CacheFile,
    notice: Arc<Notice>,
    state: Mutex<SlotState>,
}

impl Slot {
    fn new(hub: HubId, file: CacheFile, notice: Arc<Notice>) -> Self {
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

    /// Changes the cached account and writes it through. Nothing happens without a cache.
    pub(crate) fn update_cache(
        &self,
        change: impl FnOnce(&mut AccountCache),
    ) -> Result<(), CacheError> {
        let mut state = self.state();
        let Some(cache) = state.cache.as_mut() else {
            return Ok(());
        };
        change(cache);
        self.file.save(cache)
    }

    /// Opens the vault with `keys`, remembering `cache` on disk. A cache that cannot be
    /// written is logged: the account still works, but cannot unlock without the hub.
    pub(crate) fn install(&self, keys: OpenKeys, cache: AccountCache) {
        {
            let mut state = self.state();
            if let Err(error) = self.file.save(&cache) {
                tracing::warn!(%error, hub = %self.hub, "the account cache cannot be written");
            }
            state.cache = Some(cache);
            state.keys = KeyState::Open(Arc::new(keys));
        }
        self.notice.raise();
    }

    /// Opens the vault with `keys` from a cache that is already kept.
    pub(crate) fn unlocked(&self, keys: OpenKeys) {
        self.state().keys = KeyState::Open(Arc::new(keys));
        self.notice.raise();
    }

    fn await_recovery(&self, recovery: Recovery) {
        self.state().keys = KeyState::Recovering(Arc::new(recovery));
        self.notice.raise();
    }

    fn recovery(&self) -> Option<Arc<Recovery>> {
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
            was
        };
        if was_unlocked {
            self.notice.raise();
        }
    }

    /// Drops the keys and erases the cache: the account is signed out of this device.
    pub(crate) fn erase(&self) -> Result<(), HubError> {
        {
            let mut state = self.state();
            state.keys = KeyState::Locked;
            state.cache = None;
        }
        self.notice.raise();
        self.file.erase().map_err(credentials)
    }
}

/// The slots of the hubs this device knows, and where their caches are kept.
pub(crate) struct Accounts {
    root: PathBuf,
    slots: Mutex<HashMap<HubId, Arc<Slot>>>,
    notice: Arc<Notice>,
}

impl Accounts {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            slots: Mutex::default(),
            notice: Arc::new(Notice::new()),
        }
    }

    pub(crate) fn notice(&self) -> &Arc<Notice> {
        &self.notice
    }

    /// The slot of a hub; its cache is read the first time.
    pub(crate) fn slot(&self, hub: &HubId) -> Result<Arc<Slot>, HubError> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = slots.get(hub) {
            return Ok(Arc::clone(slot));
        }
        let file = CacheFile::new(&self.root, hub.address()).map_err(credentials)?;
        let slot = Arc::new(Slot::new(hub.clone(), file, Arc::clone(&self.notice)));
        slots.insert(hub.clone(), Arc::clone(&slot));
        Ok(slot)
    }
}

#[cfg(test)]
mod tests;
