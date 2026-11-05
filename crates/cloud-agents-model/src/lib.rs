//! The managed (Cloud) agents of a hub, as the desktop lists and starts them.
//!
//! Owns: the vocabulary of a listing ([`CloudAgent`], [`AgentScope`], [`AgentStatus`],
//! [`AgentLimits`]); the rules that group, filter and order agents for the Cloud tab
//! ([`arrange`]); the steps a start goes through ([`StartStep`], [`PoolOffer`],
//! [`StartedAgent`]) and what a hub can refuse ([`Refusal`]); the validated form of a personal
//! agent ([`PersonalAgentDraft`]); the ports [`CloudAgentCatalog`], [`CloudAgentLauncher`],
//! [`OwnRunnerLauncher`], [`AgentSeats`] (stop or remove the caller's seat), [`AgentUsage`] and
//! [`PersonalAgents`] with their typed
//! [`CloudAgentError`]; the chat port [`CloudChatLauncher`] (create, list with a
//! [`ChatSummary`] per row, and open Cloud chats) with its typed [`CloudChatError`].
//!
//! Does not own: the hub protocol, sealing and signatures (`nocterm-hub-io` is the adapter),
//! the catalog entity and the order of a start (`nocterm-cloud-agents`), views.
//!
//! Ports provided: `CloudAgentCatalog`, `CloudAgentLauncher`, `OwnRunnerLauncher`,
//! `AgentSeats`, `AgentUsage`, `PersonalAgents`, `CloudChatLauncher`, `ChatSshCertifier`. Ports used: `nocterm-hubs-model`
//! (`HubId`, `HubColor`, `HubError`), the `Secret` of `nocterm-session` and the
//! `CloudConversation` port of `nocterm-cloud-conversation-model`.
//!
//! Settings: none. Globals: none.
//!
//! Invariants: no I/O and no protocol types; a listing is descriptive, behaviour comes from
//! the sealed definition; a start never registers a runner for a pool whose keys are not pinned
//! or changed; a provider key is write-only and redacted in `Debug`; sections never mix hubs.
//!
//! With the `test-support` feature, [`testing::FakeCloudAgents`] implements the agent ports
//! and [`testing::FakeCloudChatLauncher`] the chat port in memory for consumers' tests.

mod agent;
mod chats;
mod error;
mod ids;
mod personal;
mod placement;
mod ports;
mod scope;
mod sections;
mod start;
mod status;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
mod usage;

pub use agent::{AgentLimits, CloudAgent, PoolLabel};
pub use chats::{
    ChatActivity, ChatFuture, ChatHistoryEntry, ChatHost, ChatListing, ChatRunner, ChatSummary,
    CloudChatError, CloudChatLauncher, OpenedChat,
};
pub use error::{CloudAgentError, ReplyProblem};
pub use ids::{AgentId, IdError, InstanceId, MAX_ID_BYTES, PoolId};
pub use personal::{
    DraftError, KeyInput, MAX_ENDPOINT_BYTES, MAX_INSTRUCTIONS_BYTES, MAX_KEY_BYTES,
    MAX_MODEL_BYTES, MAX_NAME_CHARS, PersonalAgentDraft, ProviderKind,
};
pub use placement::Placement;
pub use ports::{
    AgentHints, AgentSeats, AgentUsage, ChatSshCertifier, CloudAgentCatalog, CloudAgentLauncher,
    CloudFuture, OwnRunnerLauncher, OwnRunnerSeat, PersonalAgents, PinnedPool, PoolChoice,
    PoolPinStore, SaveOutcome, StartOutcome, StepSink, web_panel_url,
};
pub use scope::AgentScope;
pub use sections::{HubAgents, Section, SectionTitle, arrange};
pub use start::{PoolOffer, Refusal, SetupProblem, StartStep, StartedAgent};
pub use status::AgentStatus;
pub use usage::{ModelUsage, UsageSample};

#[cfg(test)]
mod tests;
