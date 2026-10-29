//! The open keys of the account, per hub, kept in memory only while the vault is unlocked.
//!
//! A hub's [`Slot`] holds the keyset opened with the vault password (never the password or
//! the unlock key beyond opening it), the collection keys opened from it, and a copy of the
//! encrypted cache file. Locking drops the keys, which wipes them. One notice is raised for
//! every change so a vault can report its status without polling.

mod admin_keys;
mod flow;
pub(crate) mod key_admission;
mod keyset_hub;
mod lifecycle;
mod recovery;
mod slot;
pub(crate) use slot::Slot;
pub(crate) mod operations;
pub(crate) mod source;
#[cfg(test)]
pub(crate) mod testing;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nocterm_hub_client::Keyset;
use nocterm_hubs_model::{HubError, HubId};
use nocterm_keys::{
    CollectionKey, OpenError, PinError, PinnedKeys, PublicKey, TrustedKey, UnlockKdf, UnlockKey,
};
use tokio::sync::watch;

use self::admin_keys::{AdminPins, Admins};
use crate::cache::{CacheFile, UnlockMethod};
use crate::classify::{credentials, remote};

pub use admin_keys::{AdminKeyError, AdminKeys, PendingAdminKey};
pub(crate) use flow::KeyFlow;
pub(crate) use lifecycle::SlotReceipt;

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

/// The opened keyset, the collection keys derived from it and the administrator keys this
/// device trusts.
pub(crate) struct OpenKeys {
    keyset: Keyset,
    collections: Mutex<HashMap<String, Arc<CollectionKey>>>,
    admins: Admins,
    /// Raised when the pending administrators change; set once the slot keeps the keys.
    notice: Option<Arc<Notice>>,
}

impl OpenKeys {
    /// The keys, trusting the administrators pinned so far.
    pub(crate) fn pinning(keyset: Keyset, pinned: PinnedKeys) -> Self {
        Self {
            keyset,
            collections: Mutex::default(),
            admins: Mutex::new(AdminPins::new(pinned)),
            notice: None,
        }
    }

    /// The keys, telling `notice` when the administrators waiting for confirmation change.
    fn noticed_by(mut self, notice: &Arc<Notice>) -> Self {
        self.notice = Some(Arc::clone(notice));
        self
    }

    fn collections(&self) -> MutexGuard<'_, HashMap<String, Arc<CollectionKey>>> {
        self.collections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn admins(&self) -> MutexGuard<'_, AdminPins> {
        admin_keys::lock(&self.admins)
    }

    /// The opened keyset.
    pub(crate) fn keyset(&self) -> &Keyset {
        &self.keyset
    }

    /// The key of a collection of the account's own: the envelope the hub holds was sealed by
    /// this account, for its personal collections and for any it created. An envelope that
    /// does not open is [`OpenError`]: the hub altered it, or it is a grant, which the hub must
    /// not pass off as the account's own.
    pub(crate) fn own_collection(
        &self,
        collection_id: &str,
        envelope: &[u8],
    ) -> Result<Arc<CollectionKey>, OpenError> {
        self.remembered(collection_id, || {
            self.keyset.open_own_collection_key(collection_id, envelope)
        })
    }

    /// The key of a collection an administrator granted, opened only under the key of that
    /// administrator as pinned on this device.
    pub(crate) fn granted_collection(
        &self,
        collection_id: &str,
        envelope: &[u8],
        granter: &TrustedKey,
    ) -> Result<Arc<CollectionKey>, OpenError> {
        self.remembered(collection_id, || {
            self.keyset
                .open_granted_collection_key(collection_id, envelope, granter)
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

    /// The key the hub offers for `account_id`, if it is the one pinned for it.
    pub(crate) fn trust(
        &self,
        account_id: &str,
        offered: &PublicKey,
    ) -> Result<TrustedKey, PinError> {
        self.admins().trust(account_id, offered)
    }

    /// Notes the administrators met unpinned in collections of `kind`, in place of the ones
    /// noted for that kind before. A change of the list raises the keys notice, so a view of
    /// the list is redrawn; looking again finds the same list and raises nothing.
    pub(crate) fn note_pending(&self, kind: &str, pending: Vec<(PendingAdminKey, PublicKey)>) {
        let changed = self.admins().replace_pending(kind, pending);
        if changed && let Some(notice) = &self.notice {
            notice.raise();
        }
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
