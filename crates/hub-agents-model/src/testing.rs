//! An in-memory hub for consumers' tests: agents, providers, the agent key and servers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};
use nocterm_hubs_model::{HubError, HubId};

use crate::{
    AgentDraft, AgentId, AgentKey, AgentsFuture, HubAgent, HubAgentDirectory, HubAgentsError,
    HubChange, HubChangeHints, HubChanges, HubServer, HubServers, NewServer, Provider,
    ProviderDraft, ProviderId, ProviderScope, ProviderTest, ServerId, ServerRemoval, ServerStatus,
};

#[derive(Default)]
struct Account {
    agents: Vec<HubAgent>,
    providers: Vec<Provider>,
    servers: Vec<HubServer>,
    key: u32,
    next: u32,
}

#[derive(Default)]
struct State {
    accounts: HashMap<HubId, Account>,
    offline: bool,
    hints: Vec<mpsc::UnboundedSender<HubChange>>,
    calls: Vec<String>,
}

/// Implements every agents port in memory. Clones share one state.
#[derive(Clone, Default)]
pub struct FakeHubAgents(Arc<Mutex<State>>);

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ready<T: Send + 'static>(value: Result<T, HubAgentsError>) -> AgentsFuture<T> {
    Box::pin(async move { value })
}

impl FakeHubAgents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a provider of the hub with `models`.
    pub fn with_provider(self, hub: &HubId, name: &str, models: &[&str]) -> Self {
        {
            let mut state = lock(&self.0);
            let account = state.accounts.entry(hub.clone()).or_default();
            account.next += 1;
            let id = ProviderId::new(format!("provider-{}", account.next)).expect("id");
            account.providers.push(Provider {
                id,
                scope: ProviderScope::Hub,
                kind: crate::ProviderKind::OpenAi,
                name: name.to_owned(),
                base_url: "https://models.example/v1".into(),
                models: models.iter().map(|model| (*model).to_owned()).collect(),
                has_key: true,
                editable: false,
            });
        }
        self
    }

    /// Adds an agent with `model` through the first provider of `hub`.
    pub fn with_agent(self, hub: &HubId, name: &str, model: &str) -> Self {
        let provider = lock(&self.0)
            .accounts
            .get(hub)
            .and_then(|account| account.providers.first())
            .map(|provider| provider.id.clone())
            .unwrap_or_else(|| ProviderId::new("provider-0").expect("id"));
        let draft = AgentDraft {
            name: name.to_owned(),
            provider: Some(provider),
            models: vec![model.to_owned()],
            ..AgentDraft::default()
        };
        futures::executor::block_on(self.create_agent(hub, draft)).expect("agent");
        self
    }

    /// Makes every call fail as if the hub could not be reached.
    pub fn set_offline(&self, offline: bool) {
        lock(&self.0).offline = offline;
    }

    /// Sets the status agents report for the server `host`.
    pub fn set_status(&self, hub: &HubId, host: &str, status: ServerStatus) {
        let mut state = lock(&self.0);
        if let Some(account) = state.accounts.get_mut(hub) {
            for server in account.servers.iter_mut().filter(|s| s.host == host) {
                server.status = status;
            }
        }
    }

    /// Tells every watcher that `change` happened, as another device would.
    pub fn changed(&self, change: HubChange) {
        lock(&self.0)
            .hints
            .retain(|hint| hint.unbounded_send(change).is_ok());
    }

    /// The names of the calls made so far, oldest first.
    pub fn calls(&self) -> Vec<String> {
        lock(&self.0).calls.clone()
    }

    fn with<T: Send + 'static>(
        &self,
        call: &str,
        hub: &HubId,
        change: impl FnOnce(&mut Account) -> Result<T, HubAgentsError>,
    ) -> AgentsFuture<T> {
        let mut state = lock(&self.0);
        state.calls.push(call.to_owned());
        if state.offline {
            return ready(Err(HubError::NotSignedIn.into()));
        }
        let account = state.accounts.entry(hub.clone()).or_default();
        ready(change(account))
    }
}

fn next_id(account: &mut Account, prefix: &str) -> String {
    account.next += 1;
    format!("{prefix}-{}", account.next)
}

fn agent_of(id: AgentId, draft: AgentDraft) -> HubAgent {
    HubAgent {
        id,
        name: draft.name,
        instructions: draft.instructions,
        provider: draft.provider.expect("validated"),
        model: draft.model,
        models: draft.models,
        approval: draft.approval,
        servers: draft.servers,
    }
}

fn provider_of(id: ProviderId, draft: ProviderDraft) -> Provider {
    Provider {
        id,
        scope: ProviderScope::Account,
        kind: draft.kind,
        name: draft.name,
        base_url: draft.base_url,
        models: draft.models,
        has_key: draft.api_key.is_some(),
        editable: true,
    }
}

fn agent_key(generation: u32) -> AgentKey {
    let public_key = format!("ssh-ed25519 AAAAkey{generation} nocterm-agents");
    AgentKey {
        authorized_line: format!(
            "no-port-forwarding,no-agent-forwarding,no-X11-forwarding {public_key}"
        ),
        public_key,
        fingerprint: format!("SHA256:key{generation}"),
    }
}

impl HubAgentDirectory for FakeHubAgents {
    fn agents(&self, hub: &HubId) -> AgentsFuture<Vec<HubAgent>> {
        self.with("agents", hub, |account| Ok(account.agents.clone()))
    }

    fn create_agent(&self, hub: &HubId, draft: AgentDraft) -> AgentsFuture<HubAgent> {
        self.with("create_agent", hub, |account| {
            let draft = draft.validate()?;
            let id = AgentId::new(next_id(account, "agent")).expect("id");
            let agent = agent_of(id, draft);
            account.agents.push(agent.clone());
            Ok(agent)
        })
    }

    fn update_agent(
        &self,
        hub: &HubId,
        agent: &AgentId,
        draft: AgentDraft,
    ) -> AgentsFuture<HubAgent> {
        let agent = agent.clone();
        self.with("update_agent", hub, move |account| {
            let draft = draft.validate()?;
            let stored = account
                .agents
                .iter_mut()
                .find(|stored| stored.id == agent)
                .ok_or(HubAgentsError::Hub(HubError::NotFound))?;
            *stored = agent_of(agent, draft);
            Ok(stored.clone())
        })
    }

    fn delete_agent(&self, hub: &HubId, agent: &AgentId) -> AgentsFuture<()> {
        let agent = agent.clone();
        self.with("delete_agent", hub, move |account| {
            account.agents.retain(|stored| stored.id != agent);
            Ok(())
        })
    }

    fn providers(&self, hub: &HubId) -> AgentsFuture<Vec<Provider>> {
        self.with("providers", hub, |account| Ok(account.providers.clone()))
    }

    fn create_provider(&self, hub: &HubId, draft: ProviderDraft) -> AgentsFuture<Provider> {
        self.with("create_provider", hub, |account| {
            let draft = draft.validate()?;
            let id = ProviderId::new(next_id(account, "provider")).expect("id");
            let provider = provider_of(id, draft);
            account.providers.push(provider.clone());
            Ok(provider)
        })
    }

    fn update_provider(
        &self,
        hub: &HubId,
        provider: &ProviderId,
        draft: ProviderDraft,
    ) -> AgentsFuture<Provider> {
        let provider = provider.clone();
        self.with("update_provider", hub, move |account| {
            let draft = draft.validate()?;
            let stored = account
                .providers
                .iter_mut()
                .find(|stored| stored.id == provider && stored.editable)
                .ok_or(HubAgentsError::Hub(HubError::NotFound))?;
            let has_key = stored.has_key || draft.api_key.is_some();
            *stored = Provider {
                has_key,
                ..provider_of(provider, draft)
            };
            Ok(stored.clone())
        })
    }

    fn delete_provider(&self, hub: &HubId, provider: &ProviderId) -> AgentsFuture<()> {
        let provider = provider.clone();
        self.with("delete_provider", hub, move |account| {
            account
                .providers
                .retain(|stored| stored.id != provider || !stored.editable);
            Ok(())
        })
    }

    fn test_provider(&self, hub: &HubId, provider: &ProviderId) -> AgentsFuture<ProviderTest> {
        let provider = provider.clone();
        self.with("test_provider", hub, move |account| {
            let stored = account
                .providers
                .iter()
                .find(|stored| stored.id == provider)
                .ok_or(HubAgentsError::Hub(HubError::NotFound))?;
            Ok(ProviderTest {
                ok: stored.has_key,
                message: if stored.has_key {
                    String::new()
                } else {
                    "The provider has no key.".into()
                },
                models: stored.models.clone(),
            })
        })
    }
}

impl HubServers for FakeHubAgents {
    fn agent_key(&self, hub: &HubId) -> AgentsFuture<AgentKey> {
        self.with("agent_key", hub, |account| Ok(agent_key(account.key)))
    }

    fn rotate_agent_key(&self, hub: &HubId) -> AgentsFuture<AgentKey> {
        self.with("rotate_agent_key", hub, |account| {
            account.key += 1;
            for server in &mut account.servers {
                server.status = ServerStatus::KeyRotated;
            }
            Ok(agent_key(account.key))
        })
    }

    fn servers(&self, hub: &HubId) -> AgentsFuture<Vec<HubServer>> {
        self.with("servers", hub, |account| Ok(account.servers.clone()))
    }

    fn add_server(&self, hub: &HubId, server: NewServer) -> AgentsFuture<HubServer> {
        self.with("add_server", hub, move |account| {
            let id = ServerId::new(next_id(account, "server")).expect("id");
            let record = HubServer {
                id,
                name: server.name,
                host: server.host,
                port: server.port,
                user: server.user,
                host_key_fingerprint: server.host_key,
                status: ServerStatus::Ready,
                status_message: String::new(),
            };
            account.servers.push(record.clone());
            Ok(record)
        })
    }

    fn check_server(&self, hub: &HubId, server: &ServerId) -> AgentsFuture<HubServer> {
        let server = server.clone();
        self.with("check_server", hub, move |account| {
            account
                .servers
                .iter()
                .find(|stored| stored.id == server)
                .cloned()
                .ok_or(HubAgentsError::Hub(HubError::NotFound))
        })
    }

    fn remove_server(&self, hub: &HubId, server: &ServerId) -> AgentsFuture<ServerRemoval> {
        let server = server.clone();
        self.with("remove_server", hub, move |account| {
            account.servers.retain(|stored| stored.id != server);
            Ok(ServerRemoval {
                line_removed: false,
                message: String::new(),
            })
        })
    }
}

struct Hints(mpsc::UnboundedReceiver<HubChange>);

impl HubChangeHints for Hints {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<HubChange>, HubAgentsError>> {
        self.0.next().map(Ok).boxed()
    }
}

impl HubChanges for FakeHubAgents {
    fn watch(&self, hub: &HubId) -> AgentsFuture<Box<dyn HubChangeHints>> {
        let (sender, receiver) = mpsc::unbounded();
        let mut state = lock(&self.0);
        state.calls.push(format!("watch {hub}"));
        if state.offline {
            return ready(Err(HubError::NotSignedIn.into()));
        }
        state.hints.push(sender);
        ready(Ok(Box::new(Hints(receiver)) as Box<dyn HubChangeHints>))
    }
}
