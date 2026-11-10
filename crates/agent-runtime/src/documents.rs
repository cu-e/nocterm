//! Documents own chat identity; shared Store owns immutable independent writes.
use super::*;
use futures::future::BoxFuture;
use gpui::AppContext as _;
use nocterm_ai::{
    execution_origin::ChatKey,
    history::{ChatSummary, ContextId, SavedChat, SharedChat},
    local_environment::ChatScope,
    storage::AgentStorageError,
};
use nocterm_store::{KeyedPersisted, KeyedRetention, StoreError};
use std::{cell::RefCell, num::NonZeroUsize, rc::Rc};

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
mod registry;
use registry::DocumentRegistry;
pub use registry::{DocumentClaim, DocumentClaimOutcome};
pub(super) struct ChatStore {
    registry: Rc<RefCell<DocumentRegistry>>,
    writer: Entity<KeyedPersisted<ChatKey, ChatIntent, CandidateStamp>>,
    catalog: crate::catalog::Catalog,
    admission: crate::history_admission::LocalHistoryAllocations,
}
impl ChatStore {
    pub(super) fn new(storage: &mut AgentStorage, cx: &mut Context<Runtime>) -> Self {
        let (admission, prepared) =
            crate::history_admission::HistoryAdmissionHandle::with_initial_payload(
                storage.has_prepared_history(),
            );
        let registry = Rc::new(RefCell::new(DocumentRegistry::new(admission.clone())));
        let owner = registry.clone();
        let runtime = cx.weak_entity();
        let repository = storage.chats();
        let backend = repository.clone();
        let writer = cx.new(|_| {
            KeyedPersisted::<ChatKey, ChatIntent, CandidateStamp>::new(
                Arc::new(move |id, intent| {
                    let backend = backend.clone();
                    Box::pin(async move {
                        match intent {
                            ChatIntent::Save(chat) => backend.save(chat).await,
                            ChatIntent::Delete => {
                                backend.delete_in(id.chat_scope(), id.id().to_owned()).await
                            }
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
                let runtime = runtime.clone();
                // A client may synchronously reenter Runtime and its writer. Deliver only
                // after the keyed entity borrow ends, and revalidate identity at delivery.
                cx.defer(move |cx| {
                    if succeeded && !retained {
                        let _ = runtime.update(cx, |runtime, cx| {
                            let _ = runtime.chats.reload(stamp.key().chat_scope(), cx);
                            cx.notify();
                        });
                    }
                    let target = registry.borrow().target(&stamp, succeeded);
                    if let Some(client) = target
                        && client
                            .state(cx)
                            .is_some_and(|state| state.origin().key() == stamp.key())
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
        let accounting = crate::history_admission::LocalHistoryAllocations::new(
            admission.clone(),
            cx.entity_id(),
        );
        Self {
            registry,
            writer,
            catalog: crate::catalog::Catalog::new(storage, admission, prepared, cx),
            admission: accounting,
        }
    }
    pub(super) fn require_client_scope(
        expected: ChatScope,
        client: &Client,
        cx: &mut App,
    ) -> Result<(), AgentStorageError> {
        let state = client.state(cx);
        if let Some(state) = &state {
            nocterm_ai::storage::require_local_scope(
                &expected,
                &state.origin().key().chat_scope(),
            )?;
        }
        if let Some((chat, _)) = client.capture(cx)? {
            nocterm_ai::storage::require_local_scope(&expected, &chat.header.chat_scope())?;
            if state.is_some_and(|state| state.origin().key().id() != chat.header.id) {
                return Err(nocterm_ai::history::HistoryCodecError::IdentityMismatch.into());
            }
        }
        Ok(())
    }
    pub(super) fn claim(
        &self,
        key: ChatKey,
        access: nocterm_ai::local_environment::LocalEnvironmentAccess,
        window: EntityId,
        replacing: Option<EntityId>,
    ) -> Result<DocumentClaimOutcome, crate::ChatOpenError> {
        DocumentRegistry::claim(&self.registry, key, access, window, replacing)
    }
    /// Commits `claim` to `client`; the owner it replaced in another window, if any.
    pub(super) fn commit(
        &mut self,
        claim: DocumentClaim,
        client: Client,
        cx: &mut App,
    ) -> Result<Option<EntityId>, crate::ChatOpenError> {
        if !claim.belongs_to(&self.registry) {
            return Err(crate::ChatOpenError::StaleClaim(claim.key().clone()));
        }
        Self::require_client_scope(claim.key().chat_scope(), &client, cx)?;
        let state = client.state(cx).ok_or(crate::ChatOpenError::Closed)?;
        if state.origin().key() != claim.key() {
            return Err(crate::ChatOpenError::Storage(
                nocterm_ai::history::HistoryCodecError::IdentityMismatch.into(),
            ));
        }
        let result = self.registry.borrow_mut().commit(&claim, client);
        // Rollback after the registry borrow ends, including failed commit.
        drop(claim);
        result
    }
    pub(super) fn take_prepared(
        &mut self,
        claim: &DocumentClaim,
        payload: crate::history_admission::HistoryPayloadPermit,
        cx: &mut App,
    ) -> Result<Option<nocterm_core::allocation::Allocated<SavedChat>>, crate::ChatOpenError> {
        if !claim.belongs_to(&self.registry) {
            return Err(crate::ChatOpenError::StaleClaim(claim.key().clone()));
        }
        self.registry.borrow().require_reservation(claim)?;
        Ok(self.catalog.take_prepared(claim.key(), payload, cx)?)
    }
    pub(super) fn require_owner_scope(
        &self,
        scope: ChatScope,
        id: EntityId,
        cx: &mut App,
    ) -> Result<(), AgentStorageError> {
        if let Some(actual) = self.registry.borrow().scope_of(id) {
            nocterm_ai::storage::require_local_scope(&scope, &actual)?;
        }
        if let Some(client) = self.client(id) {
            Self::require_client_scope(scope, &client, cx)?;
        }
        Ok(())
    }
    pub(super) fn retire(&mut self, id: EntityId, cx: &mut App) {
        let ids = self.registry.borrow_mut().retire(id);
        for key in &ids {
            self.catalog.invalidate(key.chat_scope());
        }
        self.writer.update(cx, |writer, _| {
            for id in ids {
                writer.forget_clean(&id);
            }
        });
    }
    pub(super) fn client(&self, id: EntityId) -> Option<Client> {
        self.registry.borrow().client(id)
    }
    pub(super) fn held_elsewhere(&self, scope: &ChatScope, window: EntityId) -> Vec<Client> {
        self.registry.borrow().held_elsewhere(scope, window)
    }
    pub(super) fn window_of(&self, entity: EntityId) -> Option<EntityId> {
        self.registry.borrow().window_of(entity)
    }
    pub(super) fn revoke_stale_sources(&self) -> Vec<Client> {
        self.registry.borrow_mut().revoke_stale_sources()
    }
    pub(super) fn source_current(&self, key: &ChatKey) -> bool {
        key.environment().is_this_computer() || self.registry.borrow().source_current(key)
    }
    pub(super) fn clients(&self) -> Vec<Client> {
        self.registry.borrow().clients()
    }
    pub(super) fn current(&self, id: &ChatKey, owner: EntityId) -> bool {
        self.registry.borrow().current(id, owner)
    }
    pub(super) fn save(
        &mut self,
        scope: ChatScope,
        chat: Arc<SharedChat>,
        owner: EntityId,
        revision: u64,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        nocterm_ai::storage::require_local_scope(&scope, &chat.header.chat_scope())?;
        let key = ChatKey::in_environment(scope.clone(), chat.header.id.clone());
        if !self.registry.borrow().capture_current(&key, owner)
            || !self
                .registry
                .borrow()
                .accepts_revision(&key, owner, revision)
        {
            return Ok(false);
        }
        let existing = self
            .client(owner)
            .and_then(|client| client.history_usage(cx))
            .map_or(0, |usage| usage.bytes());
        let chat = self.admission.retain_external(owner, existing, chat, cx)?;
        let id = key;
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
    fn notify_admission(&self, id: &ChatKey, owner: EntityId, revision: u64, cx: &mut App) {
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
        id: ChatKey,
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
        id: ChatKey,
        intent: ChatIntent,
        stamp: CandidateStamp,
        cx: &mut App,
    ) -> Result<(), StoreError> {
        self.writer
            .update(cx, |writer, cx| writer.update_latest(id, intent, stamp, cx))
            .map(|_| ())
    }
    pub(super) fn delete(&mut self, id: ChatKey, cx: &mut App) {
        if self.registry.borrow().closing() {
            return;
        }
        self.registry.borrow_mut().seal(id.clone());
        self.catalog.discard_prepared(&id);
        self.catalog.invalidate(id.chat_scope());
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
        if let Some((chat, revision)) = client.capture(cx)? {
            let key = ChatKey::in_environment(chat.header.chat_scope(), chat.header.id.clone());
            match self.save(key.chat_scope(), chat, id, revision, cx) {
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
    pub(super) fn snapshot(&self, scope: ChatScope) -> Option<crate::CatalogSnapshot> {
        self.catalog.snapshot(scope)
    }
    pub(super) fn oldest_catalog_payload(&self) -> Option<u64> {
        self.catalog.oldest_payload()
    }
    pub(super) fn evict_catalog_payload(&mut self) -> bool {
        self.catalog.evict_payload()
    }
    pub(super) fn loaded(
        &mut self,
        scope: ChatScope,
        generation: u64,
        rows: Result<Vec<ChatSummary>, AgentStorageError>,
        allocation: crate::catalog::CatalogAllocation,
    ) {
        let rows = rows.map(|mut rows| {
            rows.retain(|row| {
                !self
                    .registry
                    .borrow()
                    .contains_deleted(&ChatKey::in_environment(scope.clone(), row.id.clone()))
            });
            rows
        });
        self.catalog.complete(scope, generation, rows, allocation)
    }
    pub(super) fn ensure(
        &mut self,
        scope: ChatScope,
        cx: &mut Context<Runtime>,
    ) -> Result<(), crate::CatalogLoadError> {
        self.catalog.ensure(scope, cx)
    }
    pub(super) fn reload(
        &mut self,
        scope: ChatScope,
        cx: &mut Context<Runtime>,
    ) -> Result<(), crate::CatalogLoadError> {
        self.catalog.reload(scope, cx)
    }
    pub(super) fn claim_residency(
        &self,
        claim: &DocumentClaim,
        cx: &App,
    ) -> Result<crate::history_admission::HistoryAdmissionHandle, crate::ChatOpenError> {
        if !claim.belongs_to(&self.registry) {
            return Err(crate::ChatOpenError::StaleClaim(claim.key().clone()));
        }
        self.registry.borrow().require_reservation(claim)?;
        Ok(self.history_residency(claim.key().chat_scope(), None, cx)?)
    }
    pub(super) fn history_residency(
        &self,
        scope: ChatScope,
        requester: Option<EntityId>,
        _cx: &App,
    ) -> Result<crate::history_admission::HistoryAdmissionHandle, AgentStorageError> {
        let registry = self.registry.borrow();
        if registry.closing() {
            return Err(AgentStorageError::Interrupted);
        }
        if let Some(requester) = requester {
            let actual = registry
                .scope_of(requester)
                .ok_or(AgentStorageError::Interrupted)?;
            nocterm_ai::storage::require_local_scope(&scope, &actual)?;
        }
        Ok(self.admission.handle())
    }
    pub(super) fn history_admission(&self) -> crate::history_admission::HistoryAdmissionHandle {
        self.admission.handle()
    }
    pub(super) fn unavailable_reader(&self) -> HistoryReader {
        self.catalog.unavailable_reader()
    }
    pub(super) fn reader(&self, scope: ChatScope) -> HistoryReader {
        self.catalog.reader(scope)
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
            .capture_current(state.origin().key(), client.id())
        {
            return Ok(());
        }
        let Some((chat, revision)) = client.capture(cx).map_err(StoreError::backend)? else {
            return Ok(());
        };
        nocterm_ai::storage::require_local_scope(
            &state.origin().key().chat_scope(),
            &chat.header.chat_scope(),
        )
        .map_err(StoreError::backend)?;
        if !self
            .registry
            .borrow()
            .accepts_revision(state.origin().key(), client.id(), revision)
        {
            return Ok(());
        }
        let chat = self
            .admission
            .retain_external(
                client.id(),
                client.history_usage(cx).map_or(0, |usage| usage.bytes()),
                chat,
                cx,
            )
            .map_err(StoreError::backend)?;
        let id = state.origin().key().clone();
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
        self.admission.handle().close();
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
    #[cfg(test)]
    pub(crate) fn claim_document(
        &self,
        key: ChatKey,
        window: EntityId,
        replacing: Option<EntityId>,
    ) -> Result<DocumentClaimOutcome, crate::ChatOpenError> {
        self.chats.claim(
            key,
            nocterm_ai::local_environment::LocalEnvironmentAccess::this_computer(),
            window,
            replacing,
        )
    }
    pub(crate) fn claim_local_document(
        &self,
        key: ChatKey,
        access: nocterm_ai::local_environment::LocalEnvironmentAccess,
        window: EntityId,
        replacing: Option<EntityId>,
    ) -> Result<DocumentClaimOutcome, crate::ChatOpenError> {
        self.chats.claim(key, access, window, replacing)
    }
    pub(crate) fn commit_document(
        &mut self,
        claim: DocumentClaim,
        client: Client,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::ChatOpenError> {
        if let Some(chat) = self.chats.commit(claim, client, cx)? {
            cx.emit(crate::HolderEvent::Moved { chat });
        }
        cx.emit(crate::HolderEvent::Changed);
        Ok(())
    }
    /// The chats of `scope` windows other than `window` hold.
    pub(crate) fn held_elsewhere(&self, scope: &ChatScope, window: EntityId) -> Vec<Client> {
        self.chats.held_elsewhere(scope, window)
    }
    /// The window that keeps chat `owner` from moving to `window`: its own, while its agent
    /// works there or it shows there. A chat moves from one window to another only at rest.
    pub(crate) fn kept_by(&self, owner: EntityId, window: EntityId, cx: &App) -> Option<EntityId> {
        let held = self.chats.window_of(owner)?;
        if held == window {
            return None;
        }
        let state = self.chats.client(owner).and_then(|client| client.state(cx));
        state
            .is_none_or(|state| state.busy() || state.shown())
            .then_some(held)
    }
    pub(crate) fn take_prepared_chat(
        &mut self,
        claim: &DocumentClaim,
        payload: crate::history_admission::HistoryPayloadPermit,
        cx: &mut App,
    ) -> Result<Option<nocterm_core::allocation::Allocated<SavedChat>>, crate::ChatOpenError> {
        self.chats.take_prepared(claim, payload, cx)
    }
    pub(crate) fn catalog_snapshot(&self, scope: ChatScope) -> Option<crate::CatalogSnapshot> {
        self.chats.snapshot(scope)
    }
    pub fn document_is_current(&self, chat_id: &str, owner: EntityId) -> bool {
        self.scoped_document_is_current(
            &ChatKey::new(ContextId::General, chat_id.to_owned()),
            owner,
        )
    }
    pub(crate) fn scoped_document_is_current(&self, key: &ChatKey, owner: EntityId) -> bool {
        !self.shutting_down && self.chats.current(key, owner)
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
    pub(crate) fn capture_scoped_document(
        &mut self,
        scope: ChatScope,
        id: EntityId,
        cx: &mut Context<Self>,
    ) -> Result<bool, AgentPersistenceError> {
        self.chats.require_owner_scope(scope, id, cx)?;
        self.capture_and_detach_document(id, cx)
    }
    pub fn save_chat(
        &mut self,
        chat: Arc<SharedChat>,
        owner: EntityId,
        revision: u64,
        cx: &mut Context<Self>,
    ) -> Result<bool, AgentPersistenceError> {
        self.scoped_save_chat(ContextId::General.into(), chat, owner, revision, cx)
    }
    pub(crate) fn scoped_save_chat(
        &mut self,
        scope: ChatScope,
        chat: Arc<SharedChat>,
        owner: EntityId,
        revision: u64,
        cx: &mut Context<Self>,
    ) -> Result<bool, AgentPersistenceError> {
        self.chats.save(scope, chat, owner, revision, cx)
    }
    pub fn delete_chat(&mut self, id: String, cx: &mut Context<Self>) {
        self.scoped_delete_chat(ChatKey::new(ContextId::General, id), cx)
    }
    pub(crate) fn scoped_delete_chat(&mut self, key: ChatKey, cx: &mut Context<Self>) {
        let scope = key.chat_scope();
        self.chats.delete(key, cx);
        self.scoped_reload_history(scope, cx);
    }
    pub fn reload_history(&mut self, cx: &mut Context<Self>) {
        self.scoped_reload_history(ContextId::General.into(), cx)
    }
    pub(crate) fn scoped_reload_history(&mut self, scope: ChatScope, cx: &mut Context<Self>) {
        if let Err(error) = self.try_scoped_reload_history(scope, cx) {
            tracing::warn!(%error, "could not request agent history");
        }
    }
    pub(crate) fn try_scoped_reload_history(
        &mut self,
        scope: ChatScope,
        cx: &mut Context<Self>,
    ) -> Result<(), crate::CatalogLoadError> {
        self.chats.reload(scope, cx)
    }
}
#[cfg(test)]
mod history_tests;
