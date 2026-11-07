//! The ports through which the desktop reaches the agents and agent servers of a hub.
//!
//! Each call names the hub; the adapter uses the account this device is signed in to there.
//! None needs the vault: agents, providers, servers and chats never wait for an unlock.

use futures::future::BoxFuture;
use nocterm_hubs_model::HubId;

use crate::agent::{AgentDraft, HubAgent};
use crate::error::HubAgentsError;
use crate::ids::{AgentId, ProviderId, ServerId};
use crate::provider::{Provider, ProviderDraft, ProviderTest};
use crate::server::{AgentKey, HubServer, NewServer, ServerRemoval};

/// A call of an agents port.
pub type AgentsFuture<T> = BoxFuture<'static, Result<T, HubAgentsError>>;

/// The account's agents and the providers they may use.
pub trait HubAgentDirectory: Send + Sync {
    fn agents(&self, hub: &HubId) -> AgentsFuture<Vec<HubAgent>>;
    fn create_agent(&self, hub: &HubId, draft: AgentDraft) -> AgentsFuture<HubAgent>;
    fn update_agent(
        &self,
        hub: &HubId,
        agent: &AgentId,
        draft: AgentDraft,
    ) -> AgentsFuture<HubAgent>;
    /// Deletes the agent and its chats.
    fn delete_agent(&self, hub: &HubId, agent: &AgentId) -> AgentsFuture<()>;

    /// The hub's providers and the account's own.
    fn providers(&self, hub: &HubId) -> AgentsFuture<Vec<Provider>>;
    /// Adds a provider of the account's own.
    fn create_provider(&self, hub: &HubId, draft: ProviderDraft) -> AgentsFuture<Provider>;
    fn update_provider(
        &self,
        hub: &HubId,
        provider: &ProviderId,
        draft: ProviderDraft,
    ) -> AgentsFuture<Provider>;
    fn delete_provider(&self, hub: &HubId, provider: &ProviderId) -> AgentsFuture<()>;
    /// Calls the provider from an agent worker and lists the models it offers.
    fn test_provider(&self, hub: &HubId, provider: &ProviderId) -> AgentsFuture<ProviderTest>;
}

/// The account's agent key and the servers connected to its agents.
pub trait HubServers: Send + Sync {
    /// The account's agent key; the hub creates it on first use.
    fn agent_key(&self, hub: &HubId) -> AgentsFuture<AgentKey>;
    /// Replaces the agent key; every server needs its line written again.
    fn rotate_agent_key(&self, hub: &HubId) -> AgentsFuture<AgentKey>;
    fn servers(&self, hub: &HubId) -> AgentsFuture<Vec<HubServer>>;
    /// Records a server whose `authorized_keys` already holds the agent key's line.
    fn add_server(&self, hub: &HubId, server: NewServer) -> AgentsFuture<HubServer>;
    /// Signs in with the agent key and updates the status.
    fn check_server(&self, hub: &HubId, server: &ServerId) -> AgentsFuture<HubServer>;
    /// Agents stop using the server at once. The line stays on the server.
    fn remove_server(&self, hub: &HubId, server: &ServerId) -> AgentsFuture<ServerRemoval>;
}

/// What changed on a hub, here or on another device. A hint means "list it again".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubChange {
    /// Chats were created, renamed, ran or deleted.
    Chats,
    /// Agents, or the providers and models they use.
    Agents,
    /// Servers connected to agents, the agent key, or what agents last found on a server.
    Servers,
}

/// Lossy hints of what changed on a hub.
pub trait HubChangeHints: Send {
    /// Waits for the next hint; `None` when the hub ended the stream (subscribe again).
    fn next(&mut self) -> BoxFuture<'_, Result<Option<HubChange>, HubAgentsError>>;
}

/// Follows the chats, agents and servers of a hub, so every window shows what other devices
/// changed.
pub trait HubChanges: Send + Sync {
    fn watch(&self, hub: &HubId) -> AgentsFuture<Box<dyn HubChangeHints>>;
}
