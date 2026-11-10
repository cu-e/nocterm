//! Shared real-key/native-account fixtures over MemoryHub and ScriptedSubjects.
use super::{ACCOUNT_ID, PASSWORD, ScriptedSubjects, unlock};
use crate::{
    account::{OpenKeys, Slot, operations::AccountOperations, source::LocalSourceProvider},
    store::memory::{MemoryAccountStores, MemoryHub},
};
use nocterm_hub_client::Keyset;
use std::sync::Arc;

pub(crate) fn native_operations(slot: &Arc<Slot>, hub: Arc<MemoryHub>) -> Arc<AccountOperations> {
    let cached = slot.read_cache(|cache| cache.cloned());
    let was_open = slot.open().is_some();
    let account = cached
        .as_ref()
        .map_or(ACCOUNT_ID, |cache| cache.account_id.as_str());
    let subjects = Arc::new(ScriptedSubjects::for_slot(slot));
    subjects.sign_in(slot.hub().clone(), account, "device");
    if was_open && let Some(cache) = &cached {
        let keyset = Keyset::open(&cache.keyset.record(), account, &unlock(PASSWORD)).unwrap();
        slot.install(OpenKeys::new(keyset), cache.clone());
    }
    let stores = Arc::new(MemoryAccountStores::new([(account.to_owned(), hub)]));
    let provider = LocalSourceProvider::new(slot.clone(), subjects);
    Arc::new(AccountOperations::with_stores(provider, stores))
}

/// Drives the real general-vault and bound-replica ports over one MemoryHub.
pub(crate) struct NativeVault {
    vault: crate::vault::HubVault,
}
impl NativeVault {
    pub(crate) fn new(slot: Arc<Slot>, hub: Arc<MemoryHub>) -> Self {
        let operations = native_operations(&slot, hub);
        Self {
            vault: crate::vault::HubVault::new(tokio::runtime::Handle::current(), slot, operations),
        }
    }
}
impl std::ops::Deref for NativeVault {
    type Target = crate::vault::HubVault;
    fn deref(&self) -> &Self::Target {
        &self.vault
    }
}
