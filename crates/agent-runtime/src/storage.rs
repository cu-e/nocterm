//! Explicit repositories and preloaded preferences; construction performs no I/O.
use nocterm_ai::{
    favorites::AgentStateFile,
    storage::{AgentStateRepository, AgentStorageError, ChatRepository},
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AgentStorage {
    state: Arc<dyn AgentStateRepository>,
    chats: Arc<dyn ChatRepository>,
    initial: AgentStateFile,
    initial_error: Option<AgentStorageError>,
    history: Option<crate::catalog::PreparedHistory>,
}
impl AgentStorage {
    pub fn prepared(
        state: Arc<dyn AgentStateRepository>,
        chats: Arc<dyn ChatRepository>,
        initial: Result<AgentStateFile, AgentStorageError>,
    ) -> Self {
        let (initial, initial_error) = match initial {
            Ok(initial) => (initial, None),
            Err(error) => (AgentStateFile::default(), Some(error)),
        };
        Self {
            state,
            chats,
            initial,
            initial_error,
            history: None,
        }
    }
    pub fn with_history(mut self, history: crate::catalog::PreparedHistory) -> Self {
        self.history = Some(history);
        self
    }
    pub(super) fn take_history(&mut self) -> Option<crate::catalog::PreparedHistory> {
        self.history.take()
    }
    pub async fn prepare(
        state: Arc<dyn AgentStateRepository>,
        chats: Arc<dyn ChatRepository>,
    ) -> Self {
        let initial = state.load().await;
        Self::prepared(state, chats, initial)
    }
    pub(super) fn state(&self) -> Arc<dyn AgentStateRepository> {
        self.state.clone()
    }
    pub(super) fn chats(&self) -> Arc<dyn ChatRepository> {
        self.chats.clone()
    }
    pub(super) fn initial(&self) -> AgentStateFile {
        self.initial.clone()
    }
    pub(super) fn initial_error(&self) -> Option<AgentStorageError> {
        self.initial_error.clone()
    }
}

/// One shared payload permit remains held until the GUI accepts or drops a load.
#[derive(Clone)]
pub struct HistoryReader {
    repository: Arc<dyn ChatRepository>,
    gate: nocterm_ai::history::HistoryGate,
}
impl HistoryReader {
    pub(super) fn new(
        repository: Arc<dyn ChatRepository>,
        gate: nocterm_ai::history::HistoryGate,
    ) -> Self {
        Self { repository, gate }
    }
    /// Wait for the shared permit before capturing this document's original
    /// reader. The guarded identity and native task retain their physical credit
    /// through cancellation, refusal and delivery without another future Box.
    pub fn load_managed(
        &self,
        id: nocterm_ai::history::ReaderDocumentId,
        source: nocterm_ai::history::ReaderSource,
    ) -> Result<
        nocterm_ai::history::PreparedReaderLoad,
        nocterm_ai::model::json::deferred::NativeAllocationError,
    > {
        id.prepare_load(source, self.gate.clone(), self.repository.clone())
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum AgentPersistenceError {
    #[error(transparent)]
    Storage(#[from] AgentStorageError),
    #[error(transparent)]
    Worker(#[from] nocterm_store::StoreError),
}
