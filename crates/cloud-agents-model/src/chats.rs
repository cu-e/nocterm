//! The port through which the Cloud menu creates, lists and opens Cloud chats.
//!
//! Named for the menu's need: a chat is created on a host (an agent or one of the person's own
//! runners), the person's earlier chats are listed, and either comes back as a captured
//! conversation source that a window presents. Every call is for one hub.

use std::sync::Arc;

use futures::future::BoxFuture;
use nocterm_cloud_model::projection::{ProjectionError, ports::CapturedConversationSource};
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
    /// The captured, verified source of the conversation.
    pub source: Arc<dyn CapturedConversationSource>,
    /// What the window's tab is called.
    pub title: String,
}

/// An earlier chat of the account. Holding it is the only way to open it: a label alone grants
/// nothing.
pub trait ChatHistoryEntry: Send + Sync {
    /// What the history row shows.
    fn label(&self) -> &str;

    /// Opens the chat, waking its agent's seat first when it sleeps.
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
    /// The conversation source refused.
    #[error(transparent)]
    Projection(#[from] ProjectionError),
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
