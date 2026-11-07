//! The open keys of the account, per hub, kept in memory only while the vault is unlocked.
//!
//! A hub's [`Slot`] holds the keyset opened with the vault password (never the password or
//! the unlock key beyond opening it), the collection keys opened from it, and a copy of the
//! encrypted cache file. Locking drops the keys, which wipes them. One notice is raised for
//! every change so a vault can report its status without polling.
//! Forget releases registry residency but preserves a weak Slot identity while
//! a general factory still uses it. A new login promotes that cleared Slot;
//! original captures remain bound to their retired membership and open keys.

mod cache_publication;
mod flow;
pub(crate) mod key_admission;
mod keyset_hub;
mod lifecycle;
mod recovery;
mod refresh;
mod slot;
pub(crate) use slot::Slot;
pub(crate) mod operations;
pub(crate) mod source;
#[cfg(test)]
pub(crate) mod testing;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use nocterm_hub_client::Keyset;
use nocterm_hubs_model::{HubError, HubId};
use nocterm_keys::{CollectionKey, OpenError, UnlockKdf, UnlockKey};
use tokio::sync::watch;

use crate::cache::{CacheFile, CacheWork, UnlockMethod};
use crate::classify::{credentials, remote};

pub(crate) use flow::KeyFlow;
pub(crate) use lifecycle::SlotReceipt;

/// Raises a counter whenever keys are opened, dropped or replaced.
pub(crate) struct Notice(watch::Sender<u64>);

impl Notice {
    fn new() -> Self {
        Self(watch::channel(0).0)
    }

    pub(crate) fn raise(&self) {
        self.0.send_modify(|count| *count += 1);
    }

    pub(crate) fn count(&self) -> u64 {
        *self.0.borrow()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.0.subscribe()
    }
}

/// The opened keyset and the collection keys opened with it.
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

    /// The opened keyset.
    pub(crate) fn keyset(&self) -> &Keyset {
        &self.keyset
    }

    /// The key of a collection of the account: the envelope the hub holds was sealed by the
    /// account key. An envelope that does not open is [`OpenError`]: the hub altered it.
    pub(crate) fn own_collection(
        &self,
        collection_id: &str,
        envelope: &[u8],
    ) -> Result<Arc<CollectionKey>, OpenError> {
        self.remembered(collection_id, || {
            self.keyset.open_collection_key(collection_id, envelope)
        })
    }

    fn remembered(
        &self,
        collection_id: &str,
        open: impl FnOnce() -> Result<CollectionKey, OpenError>,
    ) -> Result<Arc<CollectionKey>, OpenError> {
        if let Some(key) = self.collections().get(collection_id) {
            return Ok(Arc::clone(key));
        }
        let key = Arc::new(open()?);
        self.collections()
            .insert(collection_id.to_owned(), Arc::clone(&key));
        Ok(key)
    }

    /// A new collection key of its own, with the envelope the hub stores for this account.
    pub(crate) fn new_collection(
        &self,
        collection_id: &str,
    ) -> Result<(Arc<CollectionKey>, Vec<u8>), HubError> {
        let (key, envelope) = self.generate_collection(collection_id)?;
        self.collections()
            .insert(collection_id.to_owned(), Arc::clone(&key));
        Ok((key, envelope))
    }
    /// Transient owners retain the key themselves rather than populating a cache.
    pub(crate) fn generate_collection(
        &self,
        collection_id: &str,
    ) -> Result<(Arc<CollectionKey>, Vec<u8>), HubError> {
        let key = CollectionKey::generate().map_err(remote)?;
        let envelope = self
            .keyset
            .seal_collection_key(&key, collection_id)
            .map_err(remote)?;
        Ok((Arc::new(key), envelope))
    }
}

/// A keyset the typed password does not open, kept until a recovery key opens it.
pub(crate) struct Recovery {
    account_id: String,
    unlock: UnlockKey,
    kdf: UnlockKdf,
    method: UnlockMethod,
}

/// The slots of the hubs this device knows, and where their caches are kept.
pub(crate) struct Accounts {
    root: PathBuf,
    slots: Mutex<HashMap<HubId, SlotRetention>>,
    notice: Arc<Notice>,
    work: Arc<CacheWork>,
    // Serialize first loads with eviction, outside the short map lookup lock.
    loading: Mutex<()>,
}

enum SlotRetention {
    Resident(Arc<Slot>),
    Forgotten(Weak<Slot>),
}
impl SlotRetention {
    fn upgrade(&self) -> Option<Arc<Slot>> {
        match self {
            Self::Resident(slot) => Some(slot.clone()),
            Self::Forgotten(slot) => slot.upgrade(),
        }
    }
    fn retain(&mut self) -> Option<Arc<Slot>> {
        if let Self::Resident(slot) = self {
            return Some(slot.clone());
        }
        let slot = self.upgrade()?;
        *self = Self::Resident(slot.clone());
        Some(slot)
    }
    fn alive(&self) -> bool {
        match self {
            Self::Resident(_) => true,
            Self::Forgotten(slot) => slot.strong_count() != 0,
        }
    }
}

impl Accounts {
    #[cfg(test)]
    pub(crate) fn new(root: PathBuf) -> Self {
        Self::with_runtime(root, None)
    }

    pub(crate) fn with_runtime(root: PathBuf, runtime: Option<tokio::runtime::Handle>) -> Self {
        Self {
            root,
            slots: Mutex::default(),
            notice: Arc::new(Notice::new()),
            work: CacheWork::new(runtime),
            loading: Mutex::new(()),
        }
    }

    pub(crate) fn notice(&self) -> &Arc<Notice> {
        &self.notice
    }

    /// Release residency without severing a live general account factory. A
    /// failed settings removal can still sign in through that cleared Slot.
    /// Captured authority remains retired by the preceding erase transition.
    pub(crate) fn evict_if(&self, hub: &HubId, original: &Arc<Slot>) {
        let _loading = self.loading.lock().unwrap_or_else(PoisonError::into_inner);
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.retain(|_, slot| slot.alive());
        if slots
            .get(hub)
            .and_then(SlotRetention::upgrade)
            .is_some_and(|slot| Arc::ptr_eq(&slot, original))
        {
            slots.insert(
                hub.clone(),
                SlotRetention::Forgotten(Arc::downgrade(original)),
            );
        }
    }

    pub(crate) fn loaded_slot(&self, hub: &HubId) -> Option<Arc<Slot>> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = slots.get(hub).and_then(SlotRetention::upgrade);
        if slot.is_none() {
            slots.remove(hub);
        }
        slot
    }

    fn resident_slot(&self, hub: &HubId) -> Option<Arc<Slot>> {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(hub)
            .and_then(SlotRetention::retain)
    }

    /// The slot of a hub; its cache is read the first time.
    pub(crate) fn slot(&self, hub: &HubId) -> Result<Arc<Slot>, HubError> {
        if let Some(slot) = self.resident_slot(hub) {
            return Ok(slot);
        }
        let _loading = self.loading.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = self.resident_slot(hub) {
            return Ok(slot);
        }
        let file = CacheFile::new(&self.root, hub.address())
            .map_err(credentials)?
            .with_work(self.work.clone());
        let slot = Arc::new(Slot::new(hub.clone(), file, Arc::clone(&self.notice)));
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.retain(|_, slot| slot.alive());
        slots.insert(hub.clone(), SlotRetention::Resident(slot.clone()));
        Ok(slot)
    }
}

#[cfg(test)]
mod tests;
