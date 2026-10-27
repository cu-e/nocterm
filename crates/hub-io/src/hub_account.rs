//! The encrypted data of one signed-in account, as the application uses it.

use std::sync::Arc;

use tokio::runtime::Handle;

use crate::account::Slot;
use crate::pool::Pool;
use crate::replica::HubProfileReplica;
use crate::store::{HubStore, StoreOps};
use crate::vault::HubVault;

/// A hub account's profile replica and vault. They share the keys of one account: unlocking
/// the vault opens both, and locking it (or signing out) closes both.
pub struct HubAccount {
    replica: Arc<HubProfileReplica>,
    vault: Arc<HubVault>,
}

impl HubAccount {
    pub(crate) fn new(handle: Handle, pool: Arc<Pool>, slot: Arc<Slot>) -> Self {
        let ops: Arc<dyn StoreOps> = Arc::new(HubStore::new(pool, slot.hub().clone()));
        Self {
            replica: Arc::new(HubProfileReplica::new(
                handle.clone(),
                Arc::clone(&slot),
                Arc::clone(&ops),
            )),
            vault: Arc::new(HubVault::new(handle, slot, ops)),
        }
    }

    /// The saved profiles of the account.
    pub fn replica(&self) -> Arc<HubProfileReplica> {
        Arc::clone(&self.replica)
    }

    /// The vault of the account.
    pub fn vault(&self) -> Arc<HubVault> {
        Arc::clone(&self.vault)
    }
}
