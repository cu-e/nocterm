//! Keeps this device's profiles and vault in step with the hub account the user chose.
//!
//! The choice is the `[hubs] account` setting. While a server is chosen, the saved profiles
//! replicate to and from its account and the vault is the account's; without one, profiles stay
//! local and the vault is the file of this device. Nothing restarts when the choice changes:
//! this entity follows the registry of servers and moves the two consumers.
use gpui_kit::{AppContext as _, Context, Entity, Subscription};
use nocterm_connections_model::replica::{ProfileReplica, ReplicaState};
use nocterm_connections_store::{ConnectionStore, ProfileReplication};
use nocterm_core::Paths;
use nocterm_hubs::Hubs;
use nocterm_hubs_model::{HubError, HubId};
use nocterm_store::{ShutdownCoordinator, SnapshotWriter, StoreError};
use nocterm_vault::VaultAccess;
use std::sync::Arc;

use crate::vault_selection::SelectedVault;

/// The encrypted data of one signed-in hub account.
pub(crate) struct AccountAccess {
    pub(crate) replica: Arc<dyn ProfileReplica>,
    pub(crate) vault: Arc<dyn VaultAccess>,
}

/// What the application needs to reach a hub's account.
pub(crate) trait HubAccounts: Send + Sync {
    fn open(&self, hub: &HubId) -> Result<AccountAccess, HubError>;
}

impl HubAccounts for nocterm_hub_io::HubLinks {
    fn open(&self, hub: &HubId) -> Result<AccountAccess, HubError> {
        let account = self.open_account(hub)?;
        Ok(AccountAccess {
            replica: account.replica(),
            vault: account.vault(),
        })
    }
}

/// Follows the chosen account server.
pub(crate) struct HubAccountSync {
    hubs: Entity<Hubs>,
    accounts: Arc<dyn HubAccounts>,
    vault: Option<SelectedVault>,
    replication: Entity<ProfileReplication>,
    /// The server whose account is in use now.
    attached: Option<HubId>,
    _hubs: Subscription,
}

impl HubAccountSync {
    pub(crate) fn install(
        hubs: Entity<Hubs>,
        accounts: Arc<dyn HubAccounts>,
        vault: Option<SelectedVault>,
        replication: Entity<ProfileReplication>,
        cx: &mut gpui_kit::App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(hubs, accounts, vault, replication, cx))
    }

    fn new(
        hubs: Entity<Hubs>,
        accounts: Arc<dyn HubAccounts>,
        vault: Option<SelectedVault>,
        replication: Entity<ProfileReplication>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observed = cx.observe(&hubs, |this, _, cx| this.follow(cx));
        let mut sync = Self {
            hubs,
            accounts,
            vault,
            replication,
            attached: None,
            _hubs: observed,
        };
        sync.follow(cx);
        sync
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        // The setting only names a server that is still listed.
        let wanted = self.hubs.read(cx).account_hub(cx);
        if wanted == self.attached {
            return;
        }
        if self.attached.take().is_some() {
            self.leave(cx);
        }
        let Some(hub) = wanted else {
            return;
        };
        match self.accounts.open(&hub) {
            Ok(account) => {
                if let Some(vault) = &self.vault {
                    vault.select(account.vault);
                }
                self.replication.update(cx, |replication, cx| {
                    replication.attach(account.replica, cx)
                });
                self.attached = Some(hub);
            }
            Err(error) => {
                tracing::error!(%error, "the account of the chosen server cannot be opened");
            }
        }
    }

    /// Back to this device's own profiles and vault.
    fn leave(&mut self, cx: &mut Context<Self>) {
        self.replication
            .update(cx, |replication, cx| replication.detach(cx));
        if let Some(vault) = &self.vault {
            vault.select_home();
        }
    }
}

/// The agreed state of the profile replication, kept in a private file of `paths` when the
/// application keeps files, and registered for the orderly shutdown.
pub(crate) fn replication(
    store: Entity<ConnectionStore>,
    stored: Option<&Paths>,
    shutdown: &ShutdownCoordinator,
    cx: &mut gpui_kit::App,
) -> Entity<ProfileReplication> {
    let file = stored.map(|paths| {
        Arc::new(nocterm_connections_io::FileReplicaState::new(
            paths.state_dir().join("hub-replica.json"),
        ))
    });
    let initial = file.as_ref().map(|file| file.load()).unwrap_or_default();
    let writer: SnapshotWriter<ReplicaState> = Arc::new(move |state| {
        let file = file.clone();
        Box::pin(async move {
            if let Some(file) = file {
                file.save(&state).map_err(StoreError::backend)?;
            }
            Ok(())
        })
    });
    let document = cx.new(|_| ProfileReplication::state_document(initial, writer));
    if let Err(error) = shutdown.register(&document) {
        tracing::error!(%error, "shutdown registration failed");
    }
    cx.new(|cx| ProfileReplication::new(store, document, cx))
}
