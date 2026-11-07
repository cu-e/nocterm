//! Hub chats as ordinary agent chats.
//!
//! A hub agent's chat runs on the hub; this module shows it through the neutral agent port of
//! `nocterm-ai`, so the desktop presents it with the same chat views as a local agent.
//! [`HubAgentConnector`] implements `AgentConnector` over `ChatService`: a session is a hub
//! chat (a new one is created by its first message under the session's id, a reopened one
//! reads its `History`), a prompt is `Send` and resolves when the
//! run that answers it ends, cancel is `Stop`, an approval request is a permission request
//! answered with `ResolveApproval`, and the model is a configuration option (`SetChatModel`).
//! One `Watch` per open chat follows its items and the text the agent is writing; a turn
//! someone started on another device arrives as `SessionUpdate::Turn`.
//! [`HubChatRepository`] puts the hub's chats beside the local history of a hub window and
//! sends renames, pins and deletions of hub chats to the hub.

mod connector;
mod hints;
mod items;
mod repository;
mod state;
mod upload;
mod watch;

pub use connector::HubAgentConnector;
pub(crate) use hints::ChatHints;
pub use repository::HubChatRepository;
