//! An in-memory [`CloudChatLauncher`]: runners, history and the scripted result of creating.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt as _;
use nocterm_cloud_conversation_model::testing::FakeConversation;
use nocterm_hubs_model::HubId;

use crate::chats::{
    ChatFuture, ChatHistoryEntry, ChatHost, ChatRunner, ChatSummary, CloudChatError,
    CloudChatLauncher, OpenedChat,
};

/// How a call of the fake ends instead of succeeding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatFailure {
    /// The runner did not open the chat in time.
    RunnerNotReady,
    /// The runner is not registered on this device.
    NotEnrolled,
    /// The agent has no running instance on this account.
    AgentNotStarted,
    /// The account changed meanwhile.
    Stale,
}

impl ChatFailure {
    fn error(self) -> CloudChatError {
        match self {
            Self::RunnerNotReady => CloudChatError::RunnerNotReady,
            Self::NotEnrolled => CloudChatError::NotEnrolled,
            Self::AgentNotStarted => CloudChatError::AgentNotStarted,
            Self::Stale => CloudChatError::Stale,
        }
    }
}

#[derive(Default)]
struct State {
    runners: Vec<ChatRunner>,
    history: Vec<(ChatSummary, FakeConversation)>,
    failure: Option<ChatFailure>,
    created: Vec<ChatHost>,
    opened: Vec<FakeConversation>,
    listed: usize,
}

/// The chat port of fake hubs. Clones share one state, so a test keeps a handle while the code
/// under test holds another.
#[derive(Clone, Default)]
pub struct FakeCloudChatLauncher {
    state: Arc<Mutex<State>>,
}

impl FakeCloudChatLauncher {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The runners that can host a plain chat.
    pub fn set_runners(&self, runners: Vec<ChatRunner>) {
        self.state().runners = runners;
    }

    /// An earlier chat that the history lists under `title`, hosted by an agent called "Scribe"
    /// with the model "gpt-x" and no known time.
    pub fn add_history(&self, title: &str) {
        let key = format!("chat-{}", self.state().history.len());
        self.add_history_summary(ChatSummary::plain(
            key,
            title.to_owned(),
            "Scribe".to_owned(),
            Some("gpt-x".to_owned()),
        ));
    }

    /// An earlier chat that the history lists with exactly `summary`.
    pub fn add_history_summary(&self, summary: ChatSummary) {
        let source = FakeConversation::new(&summary.key);
        self.state().history.push((summary, source));
    }

    /// Creating a chat fails like this until [`Self::heal`].
    pub fn fail_creating(&self, failure: ChatFailure) {
        self.state().failure = Some(failure);
    }

    /// Creating works again.
    pub fn heal(&self) {
        self.state().failure = None;
    }

    /// The hosts chats were created on, in order, including failed attempts.
    pub fn created(&self) -> Vec<ChatHost> {
        self.state().created.clone()
    }

    /// The conversations handed to the caller, by creating or by opening a history entry.
    pub fn opened(&self) -> Vec<FakeConversation> {
        self.state().opened.clone()
    }

    /// How many times the history was listed.
    pub fn listings(&self) -> usize {
        self.state().listed
    }
}

struct FakeEntry {
    launcher: FakeCloudChatLauncher,
    summary: ChatSummary,
    source: FakeConversation,
}

impl ChatHistoryEntry for FakeEntry {
    fn summary(&self) -> &ChatSummary {
        &self.summary
    }

    fn open(&self) -> ChatFuture<OpenedChat> {
        self.launcher.state().opened.push(self.source.clone());
        let chat = OpenedChat {
            conversation: Arc::new(self.source.clone()),
            title: self.summary.title.clone(),
            key: self.summary.key.clone(),
        };
        async move { Ok(chat) }.boxed()
    }
}

impl CloudChatLauncher for FakeCloudChatLauncher {
    fn runners(&self, _hub: &HubId) -> ChatFuture<Vec<ChatRunner>> {
        let runners = self.state().runners.clone();
        async move { Ok(runners) }.boxed()
    }

    fn create(&self, _hub: &HubId, host: ChatHost) -> ChatFuture<OpenedChat> {
        let mut state = self.state();
        state.created.push(host.clone());
        let result = match state.failure {
            Some(failure) => Err(failure.error()),
            None => {
                let key = format!("created-{}", state.created.len());
                let source = FakeConversation::new(&key);
                state.opened.push(source.clone());
                let title = match host {
                    ChatHost::Agent { agent, .. } => agent.to_string(),
                    ChatHost::Runner(runner) | ChatHost::OwnAgent { runner, .. } => runner.label,
                };
                Ok(OpenedChat {
                    conversation: Arc::new(source),
                    title,
                    key,
                })
            }
        };
        async move { result }.boxed()
    }

    fn history(&self, _hub: &HubId) -> ChatFuture<Vec<Arc<dyn ChatHistoryEntry>>> {
        let mut state = self.state();
        state.listed += 1;
        let entries = state
            .history
            .iter()
            .map(|(summary, source)| {
                Arc::new(FakeEntry {
                    launcher: self.clone(),
                    summary: summary.clone(),
                    source: source.clone(),
                }) as Arc<dyn ChatHistoryEntry>
            })
            .collect();
        async move { Ok(entries) }.boxed()
    }
}
