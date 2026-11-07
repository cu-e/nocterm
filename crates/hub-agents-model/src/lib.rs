//! The agents, model providers and agent servers of a hub account, as the desktop shows and
//! edits them.
//!
//! Owns: the records of a listing ([`HubAgent`], [`Provider`], [`HubServer`], [`AgentKey`]),
//! the validated forms people fill in ([`AgentDraft`], [`ProviderDraft`], [`NewServer`]) with
//! their rules ([`DraftError`]), the server states agents report ([`ServerStatus`]), the ports
//! the hub adapter implements ([`HubAgentDirectory`], [`HubServers`], [`HubChanges`]) and
//! their typed error ([`HubAgentsError`]).
//! Does not own: the hub protocol (`nocterm-hub-io` is the adapter), the per-hub lists and their
//! refresh (`nocterm-hub-agents`), chats and their transcript (an ordinary agent chat over the
//! hub's `AgentConnector`), views.
//! Ports provided: `HubAgentDirectory`, `HubServers`, `HubChanges`. Ports used:
//! `nocterm-hubs-model` (`HubId`, `HubError`) and the `Secret` of `nocterm-session`.
//! Settings: none. Globals: none.
//! Invariants: no I/O and no protocol types; a provider key is write-only and never formatted;
//! an agent's default model is one of its models; a draft that breaks a rule never reaches a
//! port.
//!
//! With the `test-support` feature, [`testing::FakeHubAgents`] implements every port in memory
//! for consumers' tests.

mod agent;
mod error;
mod ids;
mod ports;
mod provider;
mod server;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use agent::{AgentDraft, ApprovalMode, HubAgent, MAX_INSTRUCTIONS_BYTES, MAX_NAME_CHARS};
pub use error::{DraftError, HubAgentsError};
pub use ids::{AgentId, IdError, MAX_ID_BYTES, ProviderId, ServerId};
pub use ports::{
    AgentsFuture, HubAgentDirectory, HubChange, HubChangeHints, HubChanges, HubServers,
};
pub use provider::{Provider, ProviderDraft, ProviderKind, ProviderScope, ProviderTest};
pub use server::{AgentKey, HubServer, NewServer, ServerRemoval, ServerStatus};

#[cfg(test)]
mod tests;
