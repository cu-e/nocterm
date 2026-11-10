//! The agents, agent servers and chat lists of the signed-in hubs, shared by every window.
//!
//! Owns: one entry per tracked hub with its agents and agent servers (listed when the hub is
//! tracked and again on [`HubAgents::refresh`]), the follow of its chat list (the hub's change
//! hints, re-subscribed with growing delays), and the launches of its agents as ordinary
//! agents of `nocterm-ai`, so a hub window's new-chat menu offers them beside the local ones,
//! and Agent access of a saved server ([`AgentTarget`]): whether a session to it is open, the
//! agent key's line written to or removed from its `authorized_keys` through that session, and
//! the hub's record of it.
//! Does not own: the hub protocol (the injected ports are implemented by `nocterm-hub-io`),
//! the rules of a draft (`nocterm-hub-agents-model`), the chats themselves (an agent chat over
//! the hub connector), views.
//! Ports used: `HubAgentDirectory`, `HubServers` and `HubChanges` of
//! `nocterm-hub-agents-model` and `AgentHosts` of `nocterm-ssh-keys`, passed in [`Deps`]. Provides: [`Installed`] with the
//! [`HubAgents`] entity and its [`HubAgentsEvent`]s.
//! Settings: none. Globals: none.
//! Invariants: a hub that is no longer tracked forgets its lists and stops its follow; a
//! failed listing keeps the last lists shown and is tried again with growing delays, and a
//! re-subscribed follow reads the lists and the chats again (hints sent meanwhile are lost); every change of a hub's lists emits
//! [`HubAgentsEvent::Changed`] once, every hint of its chat list
//! [`HubAgentsEvent::ChatsChanged`], and a hint of its agents or servers lists them again; a server is recorded on the hub only after the agent
//! key's line was written to it, with the host key this device recorded.

mod access;
mod installed;
mod service;
#[cfg(test)]
mod tests;

pub use access::{AgentAccessError, AgentTarget};
pub use installed::{Deps, Installed, install};
pub use service::{HubAgents, HubAgentsEvent};
