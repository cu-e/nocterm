//! The connections, vault and terminal capability of each hub account, built once per hub.
//!
//! A hub window shows the account of one hub: its own and its organizations' connections, kept
//! in an in-memory mirror that replicates with the hub, and the account's vault. Everything of
//! one hub is built once and shared by all of its windows ([`HubSource`]); closing a window
//! never takes it away. Removing a hub evicts its retained installation; existing windows
//! retain a retired, empty source until their workspace is released. This module is the only place that names the encrypted account data
//! ([`HubAccounts`]); the windows get plain installations.
mod agent;
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
use nocterm_hubs_model::{HubError, HubId, source_identity::LocalAccountSourceFactory as _};
use nocterm_store::SnapshotWriter;
use nocterm_vault::VaultAccess;

use follow::Follow;

type AccountReplicas = Arc<
    dyn Fn(
            Arc<dyn nocterm_hubs_model::source_identity::LocalAccountSource>,
        ) -> Result<Arc<dyn ProfileReplica>, HubError>
        + Send
        + Sync,
>;

type AccountCredentials = Arc<
    dyn Fn(
            Arc<dyn nocterm_hubs_model::source_identity::LocalAccountSource>,
        ) -> Result<Arc<dyn nocterm_session::CredentialStore>, HubError>
        + Send
        + Sync,
>;

type AccountRelay = Arc<
    dyn Fn(
            Arc<dyn nocterm_hubs_model::source_identity::LocalAccountSource>,
        ) -> Result<Arc<dyn nocterm_session::RelayDialer>, HubError>
        + Send
        + Sync,
>;

/// The encrypted data of one signed-in hub account.
pub(crate) struct AccountAccess {
    replica: AccountReplicas,
    pub(crate) vault: Arc<dyn VaultAccess>,
    source: Arc<dyn nocterm_hubs_model::source_identity::LocalAccountSourceFactory>,
    credentials: AccountCredentials,
    relay: Option<AccountRelay>,
}

impl AccountAccess {
    pub(crate) fn new(
        replica: AccountReplicas,
        vault: Arc<dyn VaultAccess>,
        source: Arc<dyn nocterm_hubs_model::source_identity::LocalAccountSourceFactory>,
        credentials: AccountCredentials,
    ) -> Self {
        Self {
            replica,
            vault,
            source,
            credentials,
            relay: None,
        }
    }
    fn with_relay(mut self, relay: AccountRelay) -> Self {
        self.relay = Some(relay);
        self
    }
}

/// What the application needs to reach a hub's account.
pub(crate) trait HubAccounts: Send + Sync {
    fn open(&self, hub: &HubId) -> Result<AccountAccess, HubError>;
}

impl HubAccounts for nocterm_hub_io::HubLinks {
    fn open(&self, hub: &HubId) -> Result<AccountAccess, HubError> {
        let account = Arc::new(self.open_account(hub)?);
        let relay_account = account.clone();
        Ok(AccountAccess::new(
            {
                let account = account.clone();
                Arc::new(move |source| {
                    account
                        .local_replica(source)
                        .map(|replica| replica as Arc<dyn ProfileReplica>)
                })
            },
            account.vault(),
            Arc::new(agent::AccountCapture::new(account.clone())),
            Arc::new(move |source| account.local_credentials(source)),
        )
        .with_relay(Arc::new(move |source| relay_account.local_relay(source))))
    }
}

/// What every window of one hub is composed from.
#[derive(Clone)]
pub(crate) struct HubSource {
    connections: nocterm_connections::Installed,
    vault: Rc<nocterm_vault_ui::Installed>,
    terminal: nocterm_terminal::Installed,
    _follow: Entity<Follow>,
    agent_source: Arc<agent::AgentSourceAvailability>,
}

impl HubSource {
    pub(crate) fn agent_source(
        &self,
    ) -> Arc<dyn nocterm_hubs_model::source_identity::LocalAccountSourceFactory> {
        self.agent_source.clone()
    }
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
    /// The agents service, through which a window's editor offers Agent access of a server.
    agents: Option<Entity<nocterm_hub_agents::HubAgents>>,
    _membership: gpui_kit::Subscription,
    sources: HashMap<HubId, HubSource>,
}

impl HubSources {
    pub(crate) fn new(
        accounts: Arc<dyn HubAccounts>,
        hubs: Entity<Hubs>,
        inputs: nocterm_connections::ConnectionInputs,
        terminal: nocterm_terminal::Installed,
        ready: nocterm_ui::UiReady,
        cx: &mut Context<Self>,
    ) -> Self {
        let membership = cx.observe(&hubs, |this, hubs, cx| {
            this.sources.retain(|hub, source| {
                if hubs.read(cx).summary(hub).is_some() {
                    return true;
                }
                source._follow.update(cx, |follow, cx| follow.retire(cx));
                false
            });
        });
        Self {
            accounts,
            hubs,
            inputs,
            terminal,
            ready,
            agents: None,
            _membership: membership,
            sources: HashMap::new(),
        }
    }

    /// The windows' editors offer Agent access of their saved servers through `agents`.
    pub(crate) fn with_agents(mut self, agents: Entity<nocterm_hub_agents::HubAgents>) -> Self {
        self.agents = Some(agents);
        self
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
        runtime: nocterm_agent_runtime::RuntimeHandle,
        cx: &mut Context<Self>,
    ) -> Result<HubSource, HubError> {
        if self.hubs.read(cx).summary(hub).is_none() {
            return Err(HubError::NotSignedIn);
        }
        if let Some(source) = self.sources.get(hub) {
            return Ok(source.clone());
        }
        let account = self.accounts.open(hub)?;
        let source = self.build(hub, account, runtime, cx)?;
        self.sources.insert(hub.clone(), source.clone());
        Ok(source)
    }

    fn build(
        &self,
        hub: &HubId,
        account: AccountAccess,
        runtime: nocterm_agent_runtime::RuntimeHandle,
        cx: &mut Context<Self>,
    ) -> Result<HubSource, HubError> {
        let AccountAccess {
            replica,
            vault,
            source,
            credentials: capture_credentials,
            relay: capture_relay,
        } = account;
        let agent_source = Arc::new(agent::AgentSourceAvailability::new(source));
        let prepared =
            nocterm_connections::PreparedConnections::volatile(self.inputs_of(hub, &vault, cx));
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
            hub_key_lock(&self.ready),
            cx,
        ));
        let credentials = crate::bootstrap::credential_provider(
            vault.clone(),
            Some(connections.connections().clone()),
            connections.credential_spaces(),
        );
        let source = agent_source.clone();
        let credentials = credentials.with_capture(move || {
            let source = source
                .capture()
                .map_err(nocterm_session::CredentialStoreError::backend)?;
            let store = capture_credentials(source.clone())
                .map_err(nocterm_session::CredentialStoreError::backend)?;
            let relay = capture_relay
                .as_ref()
                .map(|capture| capture(source.clone()))
                .transpose()
                .map_err(nocterm_session::CredentialStoreError::backend)?;
            let capture = nocterm_terminal::CredentialCapture::account(store, source);
            Ok(relay.map_or_else(
                || capture.clone(),
                |relay| capture.clone().with_relay(relay),
            ))
        });
        let terminal = self.terminal.with_credentials(credentials);
        let follow = {
            let (hubs, hub) = (self.hubs.clone(), hub.clone());
            cx.new(|cx| {
                Follow::new(
                    hub,
                    hubs,
                    follow::FollowInputs::new(
                        replica,
                        vault,
                        replication,
                        agent_source.clone(),
                        runtime,
                    ),
                    cx,
                )
            })
        };
        Ok(HubSource {
            connections,
            vault: vault_ui,
            terminal,
            _follow: follow,
            agent_source,
        })
    }

    /// The connection inputs of `hub`'s windows: the keys of its vault and, with the agents
    /// service, Agent access of its saved servers.
    fn inputs_of(
        &self,
        hub: &HubId,
        vault: &Arc<dyn VaultAccess>,
        cx: &gpui_kit::App,
    ) -> nocterm_connections::ConnectionInputs {
        let inputs = self
            .inputs
            .clone()
            .with_keys(crate::vault_keys::keys_of(vault, cx));
        match &self.agents {
            Some(agents) => inputs.with_agent_access(Arc::new(
                crate::hub_server_access::HubServerAccess::new(hub.clone(), agents.clone()),
            )),
            None => inputs,
        }
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
        let store = connections.connections().clone();
        cx.new(|cx| ProfileReplication::new(store, document, cx))
    }
}

/// The account keys of a hub follow the hub key lock policy, not the local vault's; zero keeps
/// them open until they are locked or the device signs out.
fn hub_key_lock(ready: &nocterm_ui::UiReady) -> nocterm_vault_ui::AutoLock {
    let settings = ready.settings().clone();
    Rc::new(move |cx| {
        settings
            .setting::<nocterm_hubs_model::HubsSettings>(cx)
            .key_lock()
            .unwrap_or_default()
    })
}
