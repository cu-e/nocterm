//! One deterministic repository shared by runtime, feature and composition tests.
use crate::{
    favorites::AgentStateFile,
    history::{self, ChatSummary, FileStamp, SavedChat},
    storage::{AgentStateRepository, AgentStorageError, ChatRepository, StorageFuture},
};
mod reader_task;
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
    Save(String),
    Delete(String),
}
type Release = oneshot::Receiver<Result<(), AgentStorageError>>;
#[derive(Default)]
struct State {
    preferences: AgentStateFile,
    chats: BTreeMap<String, Vec<u8>>,
    barriers: HashMap<StorageOperation, VecDeque<Release>>,
    operations: Vec<StorageOperation>,
    rejected: HashMap<StorageOperation, AgentStorageError>,
}
#[derive(Clone)]
pub struct InMemoryAgentRepository {
    retention: crate::model::retention::RetentionAuthority,
    state: Arc<Mutex<State>>,
}
impl InMemoryAgentRepository {
    pub fn new(retention: crate::model::retention::RetentionAuthority) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            retention,
        }
    }
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
            .insert(chat.id().to_owned(), bytes);
        Ok(())
    }
    /// Restorable catalog documents; encoded() still exposes empty physical records.
    pub fn chats(&self) -> Vec<SavedChat> {
        let state = self.state.lock().unwrap();
        let mut chats = state
            .chats
            .iter()
            .filter_map(|(id, bytes)| history::decode(&self.retention, bytes, id).ok())
            .filter(|chat| {
                let row = ChatSummary::from_chat(chat);
                row.has_content || row.name.is_some() || row.pinned
            })
            .collect::<Vec<_>>();
        chats.sort_by_key(|chat| {
            std::cmp::Reverse((chat.metadata().pinned, chat.metadata().updated))
        });
        chats
    }
    pub fn encoded(&self, id: &str) -> Option<Vec<u8>> {
        self.state.lock().unwrap().chats.get(id).cloned()
    }
    pub fn transcript(&self, id: &str) -> Result<SavedChat, AgentStorageError> {
        let bytes = self
            .encoded(id)
            .ok_or_else(|| AgentStorageError::NotFound(id.to_owned()))?;
        Ok(history::decode(&self.retention, &bytes, id)?)
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
    fn begin_barrier(
        &self,
        operation: StorageOperation,
    ) -> Result<Option<Release>, AgentStorageError> {
        let mut state = self.state.lock().unwrap();
        state.operations.push(operation.clone());
        if let Some(error) = state.rejected.get(&operation) {
            return Err(error.clone());
        }
        Ok(state
            .barriers
            .get_mut(&operation)
            .and_then(VecDeque::pop_front))
    }
    async fn begin(&self, operation: StorageOperation) -> Result<(), AgentStorageError> {
        if let Some(barrier) = self.begin_barrier(operation)? {
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
    fn load_managed(
        &self,
        id: history::ReaderDocumentId,
        request: history::ReaderRequest,
    ) -> Result<history::ManagedReaderFuture, history::ReaderPreparationFailure> {
        let repository = self.clone();
        request.prepare_native(id, move |id, _| {
            Ok(reader_task::ReadTask::new(repository, id))
        })
    }
    fn catalog(&self) -> StorageFuture<Vec<ChatSummary>> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::Catalog).await?;
            let state = this.state.lock().unwrap();
            let mut rows = state
                .chats
                .iter()
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
                .collect::<Vec<_>>();
            rows.sort_by_key(|row| std::cmp::Reverse((row.pinned, row.updated)));
            rows.truncate(history::MAX_RESTORED);
            Ok(rows)
        })
    }
    fn save_owned(&self, write: history::writing::ManagedWrite) -> StorageFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            let id = write.id()?.to_string();
            this.begin(StorageOperation::Save(id.clone())).await?;
            write.write(|_ledger, snapshot, intent| {
                // The fixture repository stores bytes, like its legacy route.
                // It serializes the actual borrowed physical graph directly.
                let bytes = match intent {
                    history::writing::ManagedWriteIntent::Snapshot => {
                        serde_json::to_vec(&history::physical::SnapshotWire::new(snapshot))
                            .map_err(history::HistoryCodecError::from)?
                    }
                    history::writing::ManagedWriteIntent::ArchiveDraft { updated } => {
                        let original = {
                            let state = this.state.lock().unwrap();
                            let bytes = state
                                .chats
                                .get(&id)
                                .ok_or_else(|| AgentStorageError::NotFound(id.clone()))?;
                            history::decode(&this.retention, bytes, &id)?
                        };
                        let text = snapshot.composer().draft().map(|draft| draft.to_string());
                        if original.composer().draft == text {
                            return Ok(());
                        }
                        let patch = history::ArchivedDraftPatch {
                            id: id.clone(),
                            agent_id: original.metadata().agent_id.clone(),
                            text,
                            updated,
                        };
                        history::encode(&original.with_archived_draft(&patch)?)?
                    }
                };
                this.state.lock().unwrap().chats.insert(id, bytes);
                Ok(())
            })
        })
    }
    fn delete(&self, id: String) -> StorageFuture<()> {
        let this = self.clone();
        Box::pin(async move {
            this.begin(StorageOperation::Delete(id.clone())).await?;
            this.state.lock().unwrap().chats.remove(&id);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        history::{
            ChatMetadata, ComposerData, ResumeDescriptor, TranscriptData, physical::RowBody,
        },
        model::retention::RetentionAuthority,
        thread::{Entry, ThreadState},
    };
    use futures::FutureExt;
    #[test]
    fn blocked_fixture_load_cancellation_returns_original_reader_and_keeps_fixture_identity() {
        let authority = RetentionAuthority::new(history::MAX_RESIDENT_BYTES).unwrap();
        let repository = InMemoryAgentRepository::new(authority.clone());
        let id = "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c";
        let fixture = SavedChat::from_parts(
            &authority,
            id.into(),
            ChatMetadata {
                agent_id: "provider".into(),
                title: None,
                name: None,
                pinned: false,
                updated: 7,
                model: None,
            },
            ResumeDescriptor::default(),
            TranscriptData {
                entries: vec![Entry::Agent("original fixture".into())],
                times: vec![7],
            },
            ComposerData::default(),
        )
        .unwrap();
        repository.seed_chat(&fixture).unwrap();
        let mut state = ThreadState::new(&authority, "provider", id, 1).unwrap();
        let release = repository.block_next(StorageOperation::Load(id.into()));
        let reader = state.live_mut().prepare_reader_id().unwrap();
        let mut pending = repository
            .load_managed(reader, state.reader().capture().unwrap())
            .unwrap();
        assert!(
            pending
                .poll_unpin(&mut std::task::Context::from_waker(
                    &futures::task::noop_waker()
                ))
                .is_pending()
        );
        drop(pending);
        drop(release);
        let reader = state.live_mut().prepare_reader_id().unwrap();
        let delivered = futures::executor::block_on(
            repository
                .load_managed(reader, state.reader().capture().unwrap())
                .unwrap(),
        );
        assert!(
            delivered.error().is_none(),
            "fixture managed intake: {:?}",
            delivered.error()
        );
        state.live_mut().install_delivery(delivered).unwrap();
        let live = state.live().role();
        assert_eq!(&*live.snapshot().unwrap().id(), id);
        let RowBody::Agent(text) = live.snapshot().unwrap().rows()[0].view() else {
            panic!("fixture agent")
        };
        assert_eq!(&*text.read(), "original fixture");
    }
}
