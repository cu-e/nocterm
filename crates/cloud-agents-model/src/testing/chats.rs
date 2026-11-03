//! An in-memory [`CloudChatLauncher`]: runners, history and the scripted result of creating.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt as _;
use nocterm_cloud_model::projection::testing::{ProjectionFixture, ScriptedConversationSource};
use nocterm_hubs_model::HubId;

use crate::chats::{
    ChatFuture, ChatHistoryEntry, ChatHost, ChatRunner, CloudChatError, CloudChatLauncher,
    OpenedChat,
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
    history: Vec<(String, Arc<ScriptedConversationSource>)>,
    failure: Option<ChatFailure>,
    created: Vec<ChatHost>,
    opened: Vec<Arc<ScriptedConversationSource>>,
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

    /// An earlier chat that the history lists under `label`.
    pub fn add_history(&self, label: &str) {
        let source = Self::source();
        self.state().history.push((label.to_owned(), source));
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

    /// The sources handed to the caller, by creating or by opening a history entry.
    pub fn opened(&self) -> Vec<Arc<ScriptedConversationSource>> {
        self.state().opened.clone()
    }

    /// How many times the history was listed.
    pub fn listings(&self) -> usize {
        self.state().listed
    }

    fn source() -> Arc<ScriptedConversationSource> {
        Arc::new(ScriptedConversationSource::new(
            ProjectionFixture::new().binding().clone(),
        ))
    }
}

struct FakeEntry {
    launcher: FakeCloudChatLauncher,
    label: String,
    source: Arc<ScriptedConversationSource>,
}

impl ChatHistoryEntry for FakeEntry {
    fn label(&self) -> &str {
        &self.label
    }

    fn open(&self) -> ChatFuture<OpenedChat> {
        self.launcher.state().opened.push(self.source.clone());
        let chat = OpenedChat {
            source: self.source.clone(),
            title: self.label.clone(),
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
                let source = Self::source();
                state.opened.push(source.clone());
                let title = match host {
                    ChatHost::Agent { agent, .. } => agent.to_string(),
                    ChatHost::Runner(runner) | ChatHost::OwnAgent { runner, .. } => runner.label,
                };
                Ok(OpenedChat { source, title })
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
            .map(|(label, source)| {
                Arc::new(FakeEntry {
                    launcher: self.clone(),
                    label: label.clone(),
                    source: source.clone(),
                }) as Arc<dyn ChatHistoryEntry>
            })
            .collect();
        async move { Ok(entries) }.boxed()
    }
}
