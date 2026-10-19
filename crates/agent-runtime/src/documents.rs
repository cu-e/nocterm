//! Documents own chat identity; shared Store owns immutable independent writes.
use super::*;
use futures::future::BoxFuture;
use gpui::AppContext as _;
use nocterm_ai::{
    history::{ChatSummary, SavedChat, SharedChat},
    storage::AgentStorageError,
};
use nocterm_store::{KeyedPersisted, KeyedRetention, KeyedStamp, StoreError};
use std::{cell::RefCell, collections::HashSet, num::NonZeroUsize, rc::Rc};

#[derive(Clone)]
enum ChatIntent {
    Save(Arc<SharedChat>),
    Delete,
}
#[derive(Clone)]
struct CandidateStamp {
    owner: Option<EntityId>,
    revision: u64,
    deleted: bool,
}
#[derive(Default)]
struct DocumentRegistry {
    clients: HashMap<EntityId, Client>,
    owners: HashMap<String, EntityId>,
    deleted: HashSet<String>,
    stamps: HashMap<String, (EntityId, u64)>,
    closing: bool,
}
impl DocumentRegistry {
    fn register(&mut self, client: Client, chat_id: String) {
        if self.closing || self.deleted.contains(&chat_id) {
            return;
        }
        let owner = client.id();
        self.owners.retain(|_, current| *current != owner);
        self.owners.insert(chat_id, owner);
        self.clients.insert(owner, client);
    }
    fn retire(&mut self, id: EntityId) -> Vec<String> {
        let ids = self
            .owners
            .iter()
            .filter(|(_, owner)| **owner == id)
            .map(|(id, _)| id.clone())
            .collect();
        self.clients.remove(&id);
        self.owners.retain(|_, owner| *owner != id);
        self.stamps.retain(|_, (owner, _)| *owner != id);
        ids
    }
    fn current(&self, id: &str, owner: EntityId) -> bool {
        !self.closing && self.capture_current(id, owner)
    }
    fn capture_current(&self, id: &str, owner: EntityId) -> bool {
        !self.deleted.contains(id) && self.owners.get(id) == Some(&owner)
    }
    fn accepted(&mut self, id: &str, owner: EntityId, revision: u64) {
        self.stamps.insert(id.to_owned(), (owner, revision));
    }
    fn accepts_revision(&self, id: &str, owner: EntityId, revision: u64) -> bool {
        self.capture_current(id, owner)
            && self
                .stamps
                .get(id)
                .is_none_or(|(accepted_owner, accepted_revision)| {
                    *accepted_owner != owner || revision >= *accepted_revision
                })
    }
    fn seal(&mut self, id: String) {
        self.deleted.insert(id.clone());
        self.stamps.remove(&id);
        self.owners.remove(&id);
    }
    fn target(
        &self,
        stamp: &KeyedStamp<String, CandidateStamp>,
        succeeded: bool,
    ) -> Option<Client> {
        let owner = stamp.tag().owner?;
        if stamp.tag().deleted || !self.current(stamp.key(), owner) {
            return None;
        }
        // A stale failure must not pause a newer accepted document revision.
        // Successful old publications still acknowledge their actual own revision.
        if !succeeded && self.stamps.get(stamp.key()) != Some(&(owner, stamp.tag().revision)) {
            return None;
        }
        self.clients.get(&owner).cloned()
    }
    fn client(&self, id: EntityId) -> Option<Client> {
        self.clients.get(&id).cloned()
    }
    fn clients(&self) -> Vec<Client> {
        self.clients.values().cloned().collect()
    }
    fn contains_deleted(&self, id: &str) -> bool {
        self.deleted.contains(id)
    }
    fn stop(&mut self) {
        self.closing = true;
    }
}
pub(super) struct ChatStore {
    registry: Rc<RefCell<DocumentRegistry>>,
    writer: Entity<KeyedPersisted<String, ChatIntent, CandidateStamp>>,
    catalog: crate::catalog::Catalog,
}
impl ChatStore {
    pub(super) fn new(storage: &mut AgentStorage, cx: &mut Context<Runtime>) -> Self {
        let registry = Rc::new(RefCell::new(DocumentRegistry::default()));
        let owner = registry.clone();
        let repository = storage.chats();
        let backend = repository.clone();
        let writer = cx.new(|_| {
            KeyedPersisted::<String, ChatIntent, CandidateStamp>::new(
                Arc::new(move |id, intent| {
                    let backend = backend.clone();
                    Box::pin(async move {
                        match intent {
                            ChatIntent::Save(chat) => backend.save(chat).await,
                            ChatIntent::Delete => backend.delete(id).await,
                        }
                        .map_err(StoreError::backend)
                    })
                }),
                NonZeroUsize::new(32).expect("nonzero constant"),
            )
            .with_completion_hook(move |stamp, result, cx| {
                let succeeded = result.is_ok();
                let retained = owner.borrow().target(stamp, succeeded).is_some();
                let registry = owner.clone();
                let stamp = stamp.clone();
                // A client may synchronously reenter Runtime and its writer. Deliver only
                // after the keyed entity borrow ends, and revalidate identity at delivery.
                cx.defer(move |cx| {
                    let target = registry.borrow().target(&stamp, succeeded);
                    if let Some(client) = target
                        && client
                            .state(cx)
                            .is_some_and(|state| state.chat_id == *stamp.key())
                    {
                        let event = match result {
                            Ok(()) => SessionEvent::Saved {
                                revision: stamp.tag().revision,
                            },
                            Err(error) => SessionEvent::SaveFailed {
                                revision: stamp.tag().revision,
                                error: error.into(),
                            },
                        };
                        client.emit(event, cx);
                    }
                });
                if retained {
                    KeyedRetention::Retain
                } else {
                    KeyedRetention::ForgetClean
                }
            })
        });
        Self {
            registry,
            writer,
            catalog: crate::catalog::Catalog::new(storage, cx),
        }
    }
    pub(super) fn register(&mut self, client: Client, cx: &App) {
        if let Some(state) = client.state(cx) {
            self.registry.borrow_mut().register(client, state.chat_id)
        }
    }
    pub(super) fn retire(&mut self, id: EntityId, cx: &mut App) {
        let ids = self.registry.borrow_mut().retire(id);
        self.writer.update(cx, |writer, _| {
            for id in ids {
                writer.forget_clean(&id);
            }
        });
    }
    pub(super) fn client(&self, id: EntityId) -> Option<Client> {
        self.registry.borrow().client(id)
    }
    pub(super) fn clients(&self) -> Vec<Client> {
        self.registry.borrow().clients()
    }
    pub(super) fn current(&self, id: &str, owner: EntityId) -> bool {
        self.registry.borrow().current(id, owner)
    }
    pub(super) fn save(
        &mut self,
        chat: Arc<SharedChat>,
        owner: EntityId,
        revision: u64,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        if !self.registry.borrow().current(&chat.id, owner)
            || !self
                .registry
                .borrow()
                .accepts_revision(&chat.id, owner, revision)
        {
            return Ok(false);
        }
        let id = chat.id.clone();
        self.submit(
            id.clone(),
            ChatIntent::Save(chat),
            CandidateStamp {
                owner: Some(owner),
                revision,
                deleted: false,
            },
            cx,
        )?;
        self.registry.borrow_mut().accepted(&id, owner, revision);
        self.notify_admission(&id, owner, revision, cx);
        Ok(true)
    }
    fn notify_admission(&self, id: &str, owner: EntityId, revision: u64, cx: &mut App) {
        let registry = self.registry.clone();
        let id = id.to_owned();
        cx.defer(move |cx| {
            let target = {
                let registry = registry.borrow();
                registry
                    .capture_current(&id, owner)
                    .then(|| registry.client(owner))
                    .flatten()
            };
            if let Some(client) = target {
                client.emit(SessionEvent::SaveAdmitted { revision }, cx);
            }
        });
    }
    fn notify_failed_admission(
        &self,
        id: String,
        owner: EntityId,
        revision: u64,
        error: AgentPersistenceError,
        cx: &mut App,
    ) {
        let registry = self.registry.clone();
        cx.defer(move |cx| {
            let target = {
                let registry = registry.borrow();
                registry
                    .capture_current(&id, owner)
                    .then(|| registry.client(owner))
                    .flatten()
            };
            if let Some(client) = target {
                client.emit(SessionEvent::SaveFailed { revision, error }, cx);
            }
        });
    }
    fn submit(
        &mut self,
        id: String,
        intent: ChatIntent,
        stamp: CandidateStamp,
        cx: &mut App,
    ) -> Result<(), StoreError> {
        self.writer
            .update(cx, |writer, cx| writer.update_latest(id, intent, stamp, cx))
            .map(|_| ())
    }
    pub(super) fn delete(&mut self, id: String, cx: &mut App) {
        if self.registry.borrow().closing {
            return;
        }
        self.registry.borrow_mut().seal(id.clone());
        if let Err(error) = self.submit(
            id,
            ChatIntent::Delete,
            CandidateStamp {
                owner: None,
                revision: 0,
                deleted: true,
            },
            cx,
        ) {
            tracing::warn!(%error, "agent deletion admission failed");
        }
    }
    fn capture_and_detach_document(
        &mut self,
        id: EntityId,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        let Some(client) = self.client(id) else {
            return Ok(false);
        };
        if let Some((chat, revision)) = client.capture(cx) {
            let key = chat.id.clone();
            match self.save(chat, id, revision, cx) {
                Ok(true) => {
                    let target = client.clone();
                    cx.defer(move |cx| target.emit(SessionEvent::SaveAdmitted { revision }, cx));
                }
                Ok(false) => {}
                Err(error) => {
                    self.notify_failed_admission(key, id, revision, error.clone(), cx);
                    return Err(error);
                }
            }
        }
        self.retire(id, cx);
        Ok(true)
    }
    pub(super) fn capture_and_detach_all(&mut self, cx: &mut App) -> Vec<EntityId> {
        self.clients()
            .into_iter()
            .filter_map(|client| {
                let id = client.id();
                match self.capture_and_detach_document(id, cx) {
                    Ok(true) => Some(id),
                    Ok(false) => None,
                    Err(error) => {
                        tracing::warn!(%error, "could not retain disabled agent document");
                        None
                    }
                }
            })
            .collect()
    }
    pub(super) fn take_saved(&mut self) -> Option<Vec<SavedChat>> {
        self.catalog.take_saved()
    }
    pub(super) fn take_catalog(&mut self) -> Option<Vec<ChatSummary>> {
        self.catalog.take_rows().map(|rows| {
            rows.into_iter()
                .filter(|row| !self.registry.borrow().contains_deleted(&row.id))
                .collect()
        })
    }
    pub(super) fn loaded(&mut self, rows: Result<Vec<ChatSummary>, AgentStorageError>) {
        self.catalog.complete(rows)
    }
    pub(super) fn reload(&mut self, cx: &mut Context<Runtime>) {
        self.catalog.reload(cx)
    }
    pub(super) fn reader(&self) -> HistoryReader {
        self.catalog.reader()
    }
    pub(super) fn begin_shutdown(&mut self) {
        self.registry.borrow_mut().stop();
    }
    fn capture_document(&mut self, client: &Client, cx: &mut App) -> Result<(), StoreError> {
        let Some(state) = client.state(cx) else {
            return Ok(());
        };
        if !self
            .registry
            .borrow()
            .capture_current(&state.chat_id, client.id())
        {
            return Ok(());
        }
        let Some((chat, revision)) = client.capture(cx) else {
            return Ok(());
        };
        if !self
            .registry
            .borrow()
            .accepts_revision(&chat.id, client.id(), revision)
        {
            return Ok(());
        }
        let id = chat.id.clone();
        self.submit(
            id.clone(),
            ChatIntent::Save(chat),
            CandidateStamp {
                owner: Some(client.id()),
                revision,
                deleted: false,
            },
            cx,
        )?;
        self.registry
            .borrow_mut()
            .accepted(&id, client.id(), revision);
        self.notify_admission(&id, client.id(), revision, cx);
        Ok(())
    }
    pub(super) fn capture(&mut self, cx: &mut App) -> BoxFuture<'static, Result<(), StoreError>> {
        self.begin_shutdown();
        let admission_errors = self
            .clients()
            .into_iter()
            .filter_map(|client| self.capture_document(&client, cx).err())
            .collect::<Vec<_>>();
        self.catalog.stop();
        self.registry.borrow_mut().stop();
        let captured = self.writer.update(cx, |writer, _| writer.capture());
        Box::pin(async move {
            let failure = admission_errors
                .into_iter()
                .chain(
                    captured
                        .run()
                        .await
                        .into_iter()
                        .filter_map(|outcome| outcome.result().err()),
                )
                .reduce(|first, second| StoreError::joined(Arc::new(first), Arc::new(second)));
            failure.map_or(Ok(()), Err)
        })
    }
}
impl Runtime {
    pub fn take_saved_chats(&mut self) -> Option<Vec<SavedChat>> {
        self.chats.take_saved()
    }
    pub fn take_saved_catalog(&mut self) -> Option<Vec<ChatSummary>> {
        self.chats.take_catalog()
    }
    pub fn document_is_current(&self, chat_id: &str, owner: EntityId) -> bool {
        !self.shutting_down && self.chats.current(chat_id, owner)
    }
    pub fn capture_and_detach_document(
        &mut self,
        id: EntityId,
        cx: &mut Context<Self>,
    ) -> Result<bool, AgentPersistenceError> {
        let detached = self.chats.capture_and_detach_document(id, cx)?;
        if detached {
            self.admission.withdraw(id);
        }
        Ok(detached)
    }
    pub fn save_chat(
        &mut self,
        chat: Arc<SharedChat>,
        owner: EntityId,
        revision: u64,
        cx: &mut Context<Self>,
    ) -> Result<bool, AgentPersistenceError> {
        self.chats.save(chat, owner, revision, cx)
    }
    pub fn delete_chat(&mut self, id: String, cx: &mut Context<Self>) {
        self.chats.delete(id, cx)
    }
    pub fn reload_history(&mut self, cx: &mut Context<Self>) {
        self.chats.reload(cx)
    }
}
#[cfg(test)]
mod history_tests;
