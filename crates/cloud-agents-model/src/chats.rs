//! The port through which the Cloud menu creates, lists and opens Cloud chats.
//!
//! Named for the menu's need: a chat is created on a host (an agent or one of the person's own
//! runners), the person's earlier chats are listed, and either comes back as a verified
//! conversation that a window presents. Every call is for one hub.

use std::sync::Arc;

use futures::future::BoxFuture;
use nocterm_cloud_conversation_model::{CloudConversation, CloudConversationError};
use nocterm_core::error::SharedCause;
use nocterm_hubs_model::{HubError, HubId};

use crate::error::CloudAgentError;
use crate::ids::AgentId;

/// A call of the chat port.
pub type ChatFuture<T> = BoxFuture<'static, Result<T, CloudChatError>>;

/// One of the person's own runners that can host a plain chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatRunner {
    /// The runner's identifier on the hub (16 bytes).
    pub id: [u8; 16],
    /// The name the person registered it under.
    pub label: String,
}

/// Where a new chat runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatHost {
    /// A managed agent: the chat runs on the seat the hub gives it and executes tools there.
    Agent {
        /// The agent.
        agent: AgentId,
        /// The model the chat prompts; the agent's first model is its default.
        model: String,
    },
    /// One of the person's own runners: history and controls only.
    Runner(ChatRunner),
    /// One of the person's own runners prompted with a personal agent that works only while
    /// a device is online: this device leases the agent's key to the runner (ADR 0126).
    OwnAgent {
        /// The runner that runs the chat.
        runner: ChatRunner,
        /// The personal agent whose key and model the chat uses.
        agent: AgentId,
        /// The model the chat prompts.
        model: String,
    },
}

/// A chat ready to be presented in a window.
#[derive(Clone)]
pub struct OpenedChat {
    /// The verified conversation, bound to the account it was opened for.
    pub conversation: Arc<dyn CloudConversation>,
    /// What the window's tab is called.
    pub title: String,
    /// The [`ChatSummary::key`] the history lists this chat under.
    pub key: String,
}

/// What a chat is doing, as its runner last summarized it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatActivity {
    Idle,
    Running,
    WaitingApproval,
    /// A run waits for the person to allow model use.
    NeedsGrant,
    /// A run waits for this device to lease its provider key.
    NeedsProvider,
    /// A run waits for server access.
    NeedsSsh,
    Failed,
}

/// How far a history row is confirmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatListing {
    /// The runner's summary matches the head it witnessed just now.
    Current,
    /// The runner did not answer: the row is as of its last summary.
    AsOf,
    /// The runner's chat moved on since its last summary (a running chat).
    Stale,
    /// The runner published no summary of the chat (an older runner or hub).
    Unsummarized,
}

/// What a history row of an earlier chat shows. Descriptive only: it grants nothing, and the hub
/// learns nothing from it, because it is built from the runner-signed summary sealed to the
/// account, the authenticated catalog of the chat's runner and the account's own seats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatSummary {
    /// Stable for the same chat across listings (the chat's identifier, as text), so a window
    /// can tell which row is the open chat.
    pub key: String,
    /// What the row is called.
    pub title: String,
    /// Who hosts the chat: the managed agent's name, or the name of the person's own runner.
    pub host: String,
    /// The model the chat prompts, when it can prompt.
    pub model: Option<String>,
    /// When the chat last changed (Unix seconds), from the runner's summary.
    pub updated_at: Option<u64>,
    /// What the chat is doing, from the runner's summary.
    pub activity: Option<ChatActivity>,
    /// How far the row is confirmed.
    pub listing: ChatListing,
    /// The servers the chat is bound to, as `user@host`.
    pub servers: Vec<String>,
}
impl ChatSummary {
    /// A row with nothing but its key, title, host and model.
    pub fn plain(key: String, title: String, host: String, model: Option<String>) -> Self {
        Self {
            key,
            title,
            host,
            model,
            updated_at: None,
            activity: None,
            listing: ChatListing::Unsummarized,
            servers: Vec::new(),
        }
    }
}

/// An earlier chat of the account. Holding it is the only way to open it: a summary alone grants
/// nothing.
pub trait ChatHistoryEntry: Send + Sync {
    /// What the history row shows.
    fn summary(&self) -> &ChatSummary;

    /// Opens the chat. A sleeping seat is not woken: the chat shows its stored history as
    /// unconfirmed until the person wakes it or prompts.
    fn open(&self) -> ChatFuture<OpenedChat>;
}

/// Why a chat could not be created, listed or opened.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CloudChatError {
    /// The hub, or this account, does not support Cloud chats.
    #[error("this hub does not support Cloud chats")]
    Unsupported,
    /// The runner of the chat is not registered on this device: start the agent again.
    #[error("the runner of this chat is not registered on this device")]
    NotEnrolled,
    /// The agent has no instance with a registered runner on this account: it must be started
    /// first, which also enrols its runner. Opening a chat never provisions an instance itself.
    #[error("the agent is not running; start it first")]
    AgentNotStarted,
    /// The runner did not open the chat in time; the chat appears once it does.
    #[error("the runner has not opened the chat yet")]
    RunnerNotReady,
    /// The account or hub of the call ended meanwhile.
    #[error("the account or hub changed while the call was running")]
    Stale,
    /// Too many chats or operations are open at once.
    #[error("too many Cloud chats are in use")]
    Capacity,
    /// The agent has no key on this device, so nothing can be leased to the runner.
    #[error("this device holds no key for the agent")]
    NoProviderKey,
    /// The hub refused or failed the managed agent's part.
    #[error(transparent)]
    Agent(#[from] CloudAgentError),
    /// The account cannot be reached.
    #[error(transparent)]
    Hub(#[from] HubError),
    /// The conversation refused.
    #[error(transparent)]
    Conversation(#[from] CloudConversationError),
    /// The encrypted account data or the runner's Cloud channel failed.
    #[error("the Cloud chat could not be set up")]
    Backend(#[source] SharedCause),
}

impl CloudChatError {
    /// Whether the person could try the same action again later without changing anything.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RunnerNotReady | Self::Capacity)
            || matches!(self, Self::Agent(error) if error.is_transient())
    }
}

/// Creates, lists and opens Cloud chats of the signed-in account on a hub.
pub trait CloudChatLauncher: Send + Sync {
    /// The person's own runners that are not the seat of a managed agent.
    fn runners(&self, hub: &HubId) -> ChatFuture<Vec<ChatRunner>>;

    /// Creates a chat on `host` and returns it once its runner opened it. Creating is
    /// idempotent per chat identifier; a repeated call makes a new chat.
    fn create(&self, hub: &HubId, host: ChatHost) -> ChatFuture<OpenedChat>;

    /// The chats the account's registered runners know, bounded.
    fn history(&self, hub: &HubId) -> ChatFuture<Vec<Arc<dyn ChatHistoryEntry>>>;
}
