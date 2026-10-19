//! Documents own chat identity; shared Store owns immutable independent writes.
use super::*;
use futures::future::BoxFuture;
use gpui::AppContext as _;
use nocterm_ai::{
    history::{ChatSummary, SavedChat},
    storage::AgentStorageError,
};
use nocterm_store::{KeyedPersisted, KeyedRetention, KeyedStamp, StoreError};
use std::{cell::RefCell, collections::HashSet, num::NonZeroUsize, rc::Rc};

mod ownership;
mod writer;
use ownership::ChatValues;
pub use writer::{CaptureAdmission, DocumentWriter};
type ChatDriver = KeyedPersisted<String, ChatIntent, CandidateStamp, ChatValues>;

enum ChatIntent {
    Managed(nocterm_ai::history::writing::ManagedLatest),
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
    writer: DocumentWriter,
    catalog: crate::catalog::Catalog,
}
impl ChatStore {
    pub(super) fn new(storage: &mut AgentStorage, cx: &mut Context<Runtime>) -> Self {
        let registry = Rc::new(RefCell::new(DocumentRegistry::default()));
        let writer = DocumentWriter::new(storage.chats(), registry.clone(), cx);
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
        self.writer.entity().update(cx, |writer, _| {
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
        chat: nocterm_ai::history::writing::CandidateOffer<'_>,
        owner: EntityId,
        revision: u64,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        self.writer.save(chat, owner, revision, cx)
    }
    pub(super) fn delete(&mut self, id: String, cx: &mut App) {
        self.writer.delete(id, cx);
    }
    fn notify_failed_admission(
        &self,
        id: String,
        owner: EntityId,
        revision: u64,
        error: AgentPersistenceError,
        cx: &mut App,
    ) {
        self.writer
            .notify_failed_admission(id, owner, revision, error, cx);
    }
    pub(super) fn publication(&self) -> DocumentWriter {
        self.writer.clone()
    }
    fn capture_and_detach_document(
        &mut self,
        id: EntityId,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        let Some(client) = self.client(id) else {
            return Ok(false);
        };
        let captured = match client.capture_into(&self.writer.capture_for(client.id()), cx) {
            Ok(captured) => captured,
            Err(error) => {
                let (revision, error) = error.into_failure();
                if let Some(state) = client.state(cx) {
                    self.notify_failed_admission(state.chat_id, id, revision, error.clone(), cx);
                }
                return Err(error);
            }
        };
        if let Some(revision) = captured {
            // The registry entry is retired below, so the admission is delivered to the
            // detached client itself once the Runtime borrow ends.
            let target = client.clone();
            cx.defer(move |cx| target.emit(SessionEvent::SaveAdmitted { revision }, cx));
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
        match client.capture_into(&self.writer.capture_for(client.id()), cx) {
            Ok(_) => {}
            Err(error) => {
                let (revision, error) = error.into_failure();
                self.notify_failed_admission(
                    state.chat_id,
                    client.id(),
                    revision,
                    error.clone(),
                    cx,
                );
                return Err(StoreError::backend(error));
            }
        }
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
        let captured = self
            .writer
            .entity()
            .update(cx, |writer, _| writer.capture());
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
        chat: nocterm_ai::history::writing::CandidateOffer<'_>,
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
