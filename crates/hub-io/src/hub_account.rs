//! The encrypted data of one signed-in account, as the application uses it.

use std::sync::Arc;

use tokio::runtime::Handle;

use crate::account::operations::AccountOperations;
use crate::account::source::LocalSourceProvider;
use crate::account::{AdminKeys, Slot};
use crate::pool::Pool;
use crate::replica::HubProfileReplica;
use crate::vault::{HubVault, credentials::CredentialCapture};
use nocterm_hubs_model::{HubError, source_identity::LocalAccountSource};

/// A hub account's profile replica and vault. They share the keys of one account: unlocking
/// the vault opens both, and locking it (or signing out) closes both.
pub struct HubAccount {
    handle: Handle,
    vault: Arc<HubVault>,
    admin_keys: AdminKeys,
    source: LocalSourceProvider,
    credentials: CredentialCapture,
    relay: crate::relay::RelayCapture,
    operations: Arc<AccountOperations>,
}

impl HubAccount {
    pub(crate) fn new(handle: Handle, pool: Arc<Pool>, slot: Arc<Slot>) -> Self {
        let relay = crate::relay::RelayCapture::new(handle.clone(), pool.clone());
        let source = LocalSourceProvider::new(Arc::clone(&slot), pool.clone());
        let operations = Arc::new(AccountOperations::new(source.clone(), &pool));
        let admin_keys = AdminKeys::new(operations.clone());
        let vault = Arc::new(HubVault::new(
            handle.clone(),
            Arc::clone(&slot),
            operations.clone(),
        ));
        let credentials =
            CredentialCapture::new(handle.clone(), operations.stores().clone(), &vault.idle());
        Self {
            handle,
            vault,
            admin_keys,
            source,
            credentials,
            relay,
            operations,
        }
    }

    /// The saved profiles of the account.
    pub fn replica(&self) -> Result<Arc<HubProfileReplica>, HubError> {
        self.local_replica(self.source.capture()?)
    }

    /// An immutable replica for this exact authenticated account/key owner.
    pub fn local_replica(
        &self,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<HubProfileReplica>, HubError> {
        Ok(Arc::new(HubProfileReplica::new(
            self.handle.clone(),
            self.operations.bind(source)?,
        )))
    }

    /// The administrators of the account's groups whose keys wait for a person to compare
    /// their fingerprints.
    pub fn admin_keys(&self) -> AdminKeys {
        self.admin_keys.clone()
    }

    /// The vault of the account.
    pub fn vault(&self) -> Arc<HubVault> {
        Arc::clone(&self.vault)
    }
    /// Captures this configured hub's currently signed-in, unlocked account for
    /// device-local history. Lock, sign-out or replacement login makes it stale;
    /// ordinary token refresh does not. This is not a stable Cloud identity.
    pub fn local_source(&self) -> Result<Arc<dyn LocalAccountSource>, HubError> {
        self.source.capture()
    }
    /// Relay requester tied to the original account, membership and key owner.
    pub fn local_relay(
        &self,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<dyn nocterm_session::RelayDialer>, HubError> {
        self.relay.account(&self.source, source)
    }
    /// Credentials restricted to the captured native account and the caller's
    /// current source. A stale capture never follows a replacement login.
    pub fn local_credentials(
        &self,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<dyn nocterm_session::CredentialStore>, HubError> {
        self.credentials.capture(&self.source, source)
    }
}
