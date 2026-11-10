//! One deterministic repository shared by runtime, feature and composition tests.
use crate::local_environment::ChatScope;
use crate::{
    favorites::AgentStateFile,
    history::{self, ChatSummary, FileStamp, SavedChat, SharedChat},
    storage::{
        AgentStateRepository, AgentStorageError, ChatRepository, PreparedChatBytes,
        PreparedChatLoad, StorageFuture,
    },
};
use futures::channel::oneshot;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StorageOperation {
    LoadState,
    SaveState,
    Catalog,
    Load(String),
    /// Pause after materialization while the actual decoded body retains its lease.
    Decoded(String),
    Save(String),
    Delete(String),
}
type Release = oneshot::Receiver<Result<(), AgentStorageError>>;
#[derive(Default)]
struct State {
    preferences: AgentStateFile,
    chats: HashMap<ChatScope, BTreeMap<String, Vec<u8>>>,
    barriers: HashMap<StorageOperation, VecDeque<Release>>,
    operations: Vec<StorageOperation>,
    rejected: HashMap<StorageOperation, AgentStorageError>,
}
#[derive(Clone, Default)]
pub struct InMemoryAgentRepository {
    state: Arc<Mutex<State>>,
}
impl InMemoryAgentRepository {
    pub fn seed_state(&self, state: AgentStateFile) {
        self.state.lock().unwrap().preferences = state;
    }
    pub fn state(&self) -> AgentStateFile {
        self.state.lock().unwrap().preferences.clone()
    }
    pub fn seed_chat(&self, chat: &SavedChat) -> Result<(), AgentStorageError> {
        let bytes = history::encode(chat)?;
        self.state
            .lock()
            .unwrap()
            .chats
            .entry(chat.header.chat_scope())
            .or_default()
            .insert(chat.header.id.clone(), bytes);
        Ok(())
    }
    /// Restorable catalog documents; encoded() still exposes empty physical records.
    pub fn chats(&self) -> Vec<SavedChat> {
        let state = self.state.lock().unwrap();
        let mut chats = state
            .chats
            .values()
            .flat_map(BTreeMap::iter)
            .filter_map(|(id, bytes)| history::decode(bytes, id).ok())
            .filter(|chat| {
                let row = ChatSummary::from_chat(chat);
                row.has_content || row.name.is_some() || row.pinned
            })
            .collect::<Vec<_>>();
        chats.sort_by_key(|chat| std::cmp::Reverse((chat.header.pinned, chat.header.updated)));
        chats
    }
    pub fn encoded(&self, id: &str) -> Option<Vec<u8>> {
        self.encoded_in_scope(history::ContextId::General, id)
    }
    pub fn encoded_in_scope(&self, scope: history::ContextId, id: &str) -> Option<Vec<u8>> {
        self.encoded_in_environment(&scope.into(), id)
    }
    pub fn encoded_in_environment(&self, scope: &ChatScope, id: &str) -> Option<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .chats
            .get(scope)?
            .get(id)
            .cloned()
    }
    pub fn transcript(&self, id: &str) -> Result<SavedChat, AgentStorageError> {
        self.transcript_in_scope(history::ContextId::General, id)
    }
    pub fn transcript_in_scope(
        &self,
        scope: history::ContextId,
        id: &str,
    ) -> Result<SavedChat, AgentStorageError> {
        let bytes = self
            .encoded_in_scope(scope, id)
            .ok_or_else(|| AgentStorageError::NotFound(id.to_owned()))?;
        Ok(history::decode(&bytes, id)?)
    }
    pub fn operations(&self) -> Vec<StorageOperation> {
        self.state.lock().unwrap().operations.clone()
    }
    pub fn block_next(
        &self,
        operation: StorageOperation,
    ) -> oneshot::Sender<Result<(), AgentStorageError>> {
        let (sender, receiver) = oneshot::channel();
        self.state
            .lock()
            .unwrap()
            .barriers
            .entry(operation)
            .or_default()
            .push_back(receiver);
        sender
    }
    pub fn fail_next(&self, operation: StorageOperation, error: AgentStorageError) {
        let _ = self.block_next(operation).send(Err(error));
    }
    /// Reject every attempt until the host explicitly removes the scripted fault.
    pub fn reject(&self, operation: StorageOperation, error: AgentStorageError) {
        self.state.lock().unwrap().rejected.insert(operation, error);
    }
    pub fn clear_rejection(&self, operation: &StorageOperation) {
        self.state.lock().unwrap().rejected.remove(operation);
    }
    async fn begin(&self, operation: StorageOperation) -> Result<(), AgentStorageError> {
        let barrier = {
            let mut state = self.state.lock().unwrap();
            state.operations.push(operation.clone());
            if let Some(error) = state.rejected.get(&operation) {
                return Err(error.clone());
            }
            state
                .barriers
                .get_mut(&operation)
                .and_then(VecDeque::pop_front)
        };
        if let Some(barrier) = barrier {
            barrier
                .await
                .map_err(|_| AgentStorageError::Interrupted)??;
        }
        Ok(())
    }
}
impl AgentStateRepository for InMemoryAgentRepository {
    fn load(&self) -> StorageFuture<AgentStateFile> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::LoadState).await?;
            Ok(this.state())
        })
    }
    fn save(&self, state: AgentStateFile) -> StorageFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::SaveState).await?;
            this.seed_state(state);
            Ok(())
        })
    }
}
impl ChatRepository for InMemoryAgentRepository {
    fn prepare_load_in(
        &self,
        scope: ChatScope,
        id: String,
    ) -> StorageFuture<Box<dyn PreparedChatLoad>> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::Load(id.clone())).await?;
            let bytes = {
                let state = this
                    .state
                    .lock()
                    .map_err(|_| AgentStorageError::Interrupted)?;
                state
                    .chats
                    .get(&scope)
                    .and_then(|chats| chats.get(&id))
                    .ok_or_else(|| AgentStorageError::NotFound(id.clone()))?
                    .clone()
            };
            let source = PreparedChatBytes::new(bytes, scope, id.clone(), None)?;
            Ok(Box::new(PreparedRepositoryLoad {
                source: Box::new(source),
                repository: this,
                id,
            }) as Box<dyn PreparedChatLoad>)
        })
    }
    fn catalog(&self, scope: history::ContextId) -> StorageFuture<Vec<ChatSummary>> {
        self.catalog_in(scope.into())
    }
    fn load(&self, scope: history::ContextId, id: String) -> StorageFuture<SavedChat> {
        self.load_in(scope.into(), id)
    }
    fn delete(&self, scope: history::ContextId, id: String) -> StorageFuture<()> {
        self.delete_in(scope.into(), id)
    }
    fn catalog_in(&self, scope: ChatScope) -> StorageFuture<Vec<ChatSummary>> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::Catalog).await?;
            let state = this.state.lock().unwrap();
            let mut rows = state
                .chats
                .get(&scope)
                .into_iter()
                .flat_map(BTreeMap::iter)
                .filter_map(|(id, bytes)| {
                    history::decode_summary(
                        bytes,
                        id,
                        FileStamp {
                            bytes: bytes.len() as u64,
                            modified_nanos: 0,
                        },
                    )
                    .ok()
                    .flatten()
                })
                .filter(|row| row.chat_scope() == scope)
                .collect::<Vec<_>>();
            rows.sort_by_key(|row| std::cmp::Reverse((row.pinned, row.updated)));
            rows.truncate(history::MAX_RESTORED);
            Ok(rows)
        })
    }
    fn load_in(&self, scope: ChatScope, id: String) -> StorageFuture<SavedChat> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::Load(id.clone())).await?;
            let state = this.state.lock().unwrap();
            let bytes = state
                .chats
                .get(&scope)
                .and_then(|chats| chats.get(&id))
                .ok_or_else(|| AgentStorageError::NotFound(id.clone()))?;
            let chat = history::decode(bytes, &id)?;
            crate::storage::require_local_scope(&scope, &chat.header.chat_scope())?;
            Ok(chat)
        })
    }
    fn save(&self, chat: Arc<SharedChat>) -> StorageFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::Save(chat.header.id.clone()))
                .await?;
            let bytes = if let Some((draft, updated)) = chat.metadata.draft_patch() {
                let mut original = {
                    let state = this.state.lock().unwrap();
                    let bytes = state
                        .chats
                        .get(&chat.header.chat_scope())
                        .and_then(|chats| chats.get(&chat.header.id))
                        .ok_or_else(|| AgentStorageError::NotFound(chat.header.id.clone()))?;
                    history::decode(bytes, &chat.header.id)?
                };
                crate::storage::require_local_scope(
                    &chat.header.chat_scope(),
                    &original.header.chat_scope(),
                )?;
                original.draft = draft;
                original.header.updated = updated;
                history::encode(&original)?
            } else {
                history::encode_shared(&chat)?
            };
            let mut state = this.state.lock().unwrap();
            let chats = state.chats.entry(chat.header.chat_scope()).or_default();
            if let Some(original) = chats.get(&chat.header.id) {
                let original = history::decode(original, &chat.header.id)?;
                crate::storage::require_local_scope(
                    &chat.header.chat_scope(),
                    &original.header.chat_scope(),
                )?;
            }
            chats.insert(chat.header.id.clone(), bytes);
            Ok(())
        })
    }
    fn delete_in(&self, scope: ChatScope, id: String) -> StorageFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::Delete(id.clone())).await?;
            let mut state = this.state.lock().unwrap();
            let Some(chats) = state.chats.get_mut(&scope) else {
                return Ok(());
            };
            if let Some(bytes) = chats.get(&id) {
                let chat = history::decode(bytes, &id)?;
                crate::storage::require_local_scope(&scope, &chat.header.chat_scope())?;
            }
            chats.remove(&id);
            Ok(())
        })
    }
}

struct PreparedRepositoryLoad {
    source: Box<dyn PreparedChatLoad>,
    repository: InMemoryAgentRepository,
    id: String,
}
impl PreparedChatLoad for PreparedRepositoryLoad {
    fn resident_bytes(&self) -> usize {
        self.source.resident_bytes()
    }
    fn decode(
        self: Box<Self>,
        allocation: nocterm_core::allocation::AllocationLease,
    ) -> StorageFuture<nocterm_core::allocation::Allocated<SavedChat>> {
        Box::pin(async move {
            let decoded = self.source.decode(allocation).await?;
            self.repository
                .begin(StorageOperation::Decoded(self.id))
                .await?;
            Ok(decoded)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    #[test]
    fn the_same_identity_in_two_scopes_has_independent_documents_and_deletion() {
        let repository = InMemoryAgentRepository::default();
        let general = SavedChat::for_identity(
            "agent".into(),
            "00000000-0000-4000-8000-000000000001".into(),
            1,
        );
        let mut project = general.clone();
        project.header.scope = serde_json::from_value(
            serde_json::json!({"kind":"project","id":"01010101010101010101010101010101"}),
        )
        .unwrap();
        project.header.title = Some("project caption".into());
        repository.seed_chat(&general).unwrap();
        repository.seed_chat(&project).unwrap();
        let before = repository
            .encoded_in_scope(project.header.scope, &project.header.id)
            .unwrap();
        assert_eq!(
            block_on(ChatRepository::load(
                &repository,
                general.header.scope,
                general.header.id.clone()
            ))
            .unwrap()
            .header
            .title,
            None
        );
        assert_eq!(
            block_on(ChatRepository::load(
                &repository,
                project.header.scope,
                project.header.id.clone()
            ))
            .unwrap()
            .header
            .title,
            project.header.title
        );
        block_on(repository.delete(general.header.scope, general.header.id.clone())).unwrap();
        assert!(repository.encoded(&general.header.id).is_none());
        assert_eq!(
            repository
                .encoded_in_scope(project.header.scope, &project.header.id)
                .unwrap(),
            before
        );
        assert!(matches!(
            block_on(ChatRepository::load(
                &repository,
                general.header.scope,
                general.header.id
            )),
            Err(AgentStorageError::NotFound(_))
        ));
    }
}
