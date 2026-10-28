//! The connections, vault and terminal capability of each hub account, built once per hub.
//!
//! A hub window shows the account of one hub: its own and its organizations' connections, kept
//! in an in-memory mirror that replicates with the hub, and the account's vault. Everything of
//! one hub is built once and shared by all of its windows ([`HubSource`]); closing a window
//! never takes it away. This module is the only place that names the encrypted account data
//! ([`HubAccounts`]); the windows get plain installations.
mod follow;
mod notices;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, Entity};
use nocterm_connections_model::replica::{ProfileReplica, ReplicaState};
use nocterm_connections_store::ProfileReplication;
use nocterm_core::error::SharedCause;
use nocterm_hubs::Hubs;
use nocterm_hubs_model::{HubError, HubId};
use nocterm_store::{ShutdownCoordinator, SnapshotWriter};
use nocterm_vault::VaultAccess;

use follow::Follow;

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

/// What every window of one hub is composed from.
#[derive(Clone)]
pub(crate) struct HubSource {
    connections: nocterm_connections::Installed,
    vault: Rc<nocterm_vault_ui::Installed>,
    terminal: nocterm_terminal::Installed,
    _follow: Entity<Follow>,
}

impl HubSource {
    pub(crate) fn connections(&self) -> &nocterm_connections::Installed {
        &self.connections
    }

    pub(crate) fn vault(&self) -> &Rc<nocterm_vault_ui::Installed> {
        &self.vault
    }

    pub(crate) fn terminal(&self) -> &nocterm_terminal::Installed {
        &self.terminal
    }
}

/// The sources of the hubs the windows of this run showed, one per hub.
pub(crate) struct HubSources {
    accounts: Arc<dyn HubAccounts>,
    hubs: Entity<Hubs>,
    inputs: nocterm_connections::ConnectionInputs,
    /// The terminal capability of this device, which a hub's credentials are added to.
    terminal: nocterm_terminal::Installed,
    ready: nocterm_ui::UiReady,
    shutdown: ShutdownCoordinator,
    sources: HashMap<HubId, HubSource>,
}

impl HubSources {
    pub(crate) fn new(
        accounts: Arc<dyn HubAccounts>,
        hubs: Entity<Hubs>,
        inputs: nocterm_connections::ConnectionInputs,
        terminal: nocterm_terminal::Installed,
        ready: nocterm_ui::UiReady,
        shutdown: ShutdownCoordinator,
    ) -> Self {
        Self {
            accounts,
            hubs,
            inputs,
            terminal,
            ready,
            shutdown,
            sources: HashMap::new(),
        }
    }

    /// The registry of hubs the sources follow.
    #[cfg(test)]
    pub(crate) fn registry(&self) -> &Entity<Hubs> {
        &self.hubs
    }

    /// The source of `hub`, built when it is asked for first.
    ///
    /// The mirror replicates with the hub before this returns, so the connections panel of a
    /// window built from it shows the account's state from its first frame.
    pub(crate) fn source(
        &mut self,
        hub: &HubId,
        cx: &mut Context<Self>,
    ) -> Result<HubSource, HubError> {
        if let Some(source) = self.sources.get(hub) {
            return Ok(source.clone());
        }
        let account = self.accounts.open(hub)?;
        let source = self.build(hub, account, cx)?;
        self.sources.insert(hub.clone(), source.clone());
        Ok(source)
    }

    fn build(
        &self,
        hub: &HubId,
        account: AccountAccess,
        cx: &mut Context<Self>,
    ) -> Result<HubSource, HubError> {
        let AccountAccess { replica, vault } = account;
        let keys = crate::vault_keys::keys_of(&vault, cx);
        let prepared =
            nocterm_connections::PreparedConnections::volatile(self.inputs.clone().with_keys(keys));
        let connections = nocterm_connections::install_volatile(prepared, &self.ready, cx)
            .map_err(|error| HubError::Remote {
                cause: SharedCause::new(error),
            })?;
        let replication = self.replication(&connections, cx);
        connections.bind_replication(replication.clone());
        let sign_in_again: nocterm_vault_ui::KeysMissingRemedy = {
            let (hubs, hub) = (self.hubs.clone(), hub.clone());
            Rc::new(move |window, cx| nocterm_hubs_ui::sign_in_again(&hubs, &hub, window, cx))
        };
        let vault_ui = Rc::new(nocterm_vault_ui::install_scoped(
            nocterm_vault_ui::Deps::new(vault.clone()).with_keys_missing(sign_in_again),
            &self.ready,
            cx,
        ));
        let credentials = crate::bootstrap::credential_provider(
            vault.clone(),
            Some(connections.connections().clone()),
            connections.credential_spaces(),
        );
        let terminal = self.terminal.with_credentials(credentials);
        let follow = {
            let (hubs, hub) = (self.hubs.clone(), hub.clone());
            cx.new(|cx| Follow::new(hub, hubs, replica, vault, replication, cx))
        };
        Ok(HubSource {
            connections,
            vault: vault_ui,
            terminal,
            _follow: follow,
        })
    }

    /// The replication of the mirror. What it agreed with the hub lives as long as the mirror
    /// does: a new run starts from an empty mirror and an empty agreement.
    fn replication(
        &self,
        connections: &nocterm_connections::Installed,
        cx: &mut Context<Self>,
    ) -> Entity<ProfileReplication> {
        let writer: SnapshotWriter<ReplicaState> = Arc::new(|_| Box::pin(async { Ok(()) }));
        let document =
            cx.new(|_| ProfileReplication::state_document(ReplicaState::default(), writer));
        if let Err(error) = self.shutdown.register(&document) {
            tracing::error!(%error, "shutdown registration failed");
        }
        let store = connections.connections().clone();
        cx.new(|cx| ProfileReplication::new(store, document, cx))
    }
}
