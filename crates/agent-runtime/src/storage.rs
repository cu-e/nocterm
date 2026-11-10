//! Explicit repositories and preloaded preferences; construction performs no I/O.
use nocterm_ai::{
    favorites::AgentStateFile,
    storage::{AgentStateRepository, AgentStorageError, ChatRepository},
};
use std::sync::Arc;
mod admitted;
pub use admitted::LoadedHistory;

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
    pub(super) fn has_prepared_history(&self) -> bool {
        self.history
            .as_ref()
            .is_some_and(|history| !history.records().is_empty())
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
    scope: Option<nocterm_ai::local_environment::ChatScope>,
    access: Option<nocterm_ai::local_environment::LocalEnvironmentAccess>,
}
impl HistoryReader {
    pub(super) fn new(
        repository: Arc<dyn ChatRepository>,
        gate: nocterm_ai::history::HistoryGate,
        scope: nocterm_ai::local_environment::ChatScope,
    ) -> Self {
        Self {
            repository,
            gate,
            scope: Some(scope),
            access: Some(nocterm_ai::local_environment::LocalEnvironmentAccess::this_computer()),
        }
    }
    pub(super) fn with_access(
        mut self,
        access: Option<nocterm_ai::local_environment::LocalEnvironmentAccess>,
    ) -> Self {
        self.access = access;
        self
    }
    pub(super) fn unavailable(
        repository: Arc<dyn ChatRepository>,
        gate: nocterm_ai::history::HistoryGate,
    ) -> Self {
        Self {
            repository,
            gate,
            scope: None,
            access: None,
        }
    }
    /// The captured service performs native work and admits its actual decoded
    /// generation before allocation. Cancelling this observer never frees work
    /// still owned by an independent native worker/result.
    pub fn load_admitted(
        &self,
        id: String,
        admission: crate::history_admission::HistoryAdmissionHandle,
        cx: &mut gpui::App,
    ) -> gpui::Task<LoadedHistory> {
        admitted::AdmittedHistoryLoad::new(self.clone(), id, admission).start(cx)
    }
    /// Preflight the original held source before any decoded body is allocated.
    /// The existing gate bounds raw native scratch only; it grants no decoded quota.
    pub async fn prepare_load(
        &self,
        id: String,
    ) -> (
        Result<PreparedHistoryRead, AgentStorageError>,
        nocterm_ai::history::HistoryPermit,
    ) {
        let permit = self.gate.acquire().await;
        let prepared =
            async {
                let access = self.access.as_ref().ok_or(
                    nocterm_hubs_model::source_identity::LocalAccountSourceError::Unavailable,
                )?;
                let scope = self.scope.as_ref().ok_or(
                    nocterm_hubs_model::source_identity::LocalAccountSourceError::Unavailable,
                )?;
                access.check_current()?;
                nocterm_ai::storage::require_local_environment(
                    access.environment(),
                    scope.environment(),
                )?;
                let load = self
                    .repository
                    .prepare_load_in(scope.clone(), id.clone())
                    .await?;
                access.check_current()?;
                Ok(PreparedHistoryRead {
                    load,
                    access: access.clone(),
                    scope: scope.clone(),
                    id,
                })
            }
            .await;
        (prepared, permit)
    }
}
/// Retains the exact preflight source/bytes; decode cannot reopen a pathname.
pub struct PreparedHistoryRead {
    load: Box<dyn nocterm_ai::storage::PreparedChatLoad>,
    access: nocterm_ai::local_environment::LocalEnvironmentAccess,
    scope: nocterm_ai::local_environment::ChatScope,
    id: String,
}
impl PreparedHistoryRead {
    pub fn resident_bytes(&self) -> usize {
        self.load.resident_bytes()
    }
    pub async fn decode(
        self,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> Result<
        nocterm_core::allocation::Allocated<nocterm_ai::history::SavedChat>,
        AgentStorageError,
    > {
        self.access.check_current()?;
        if allocation.payloads() == 0 || allocation.bytes() < self.load.resident_bytes() {
            return Err(AgentStorageError::ResidentCapacity {
                retained: 0,
                incoming: self.load.resident_bytes(),
                limit: allocation.bytes(),
            });
        }
        let chat = self.load.decode(allocation).await?;
        self.access.check_current()?;
        chat.value().validate_identity()?;
        if chat.value().header.id != self.id {
            return Err(nocterm_ai::history::HistoryCodecError::IdentityMismatch.into());
        }
        nocterm_ai::storage::require_local_scope(&self.scope, &chat.value().header.chat_scope())?;
        Ok(chat)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum AgentPersistenceError {
    #[error(transparent)]
    Storage(#[from] AgentStorageError),
    #[error(transparent)]
    Worker(#[from] nocterm_store::StoreError),
}
