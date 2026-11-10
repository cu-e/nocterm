//! Explicit runtime capability; all commands retain their original entity.
use crate::{
    AgentPersistenceError, Client, HistoryReader, Runtime, RuntimeServices, TerminalAuthRequest,
};
use futures::future::BoxFuture;
use gpui::{App, AppContext as _, Entity, EntityId, Subscription, Task};
use nocterm_ai::model::auth::AuthMethod;
use nocterm_ai::model::controls::ConfigOption;
use nocterm_ai::{
    AgentLaunch, BridgeRegistration,
    execution_origin::ChatKey,
    history::{ContextId, SavedChat, SharedChat},
    local_environment::{ChatScope, LocalEnvironment, LocalEnvironmentAccess},
};
use std::sync::Arc;

#[derive(Clone)]
pub struct RuntimeHandle {
    runtime: Entity<Runtime>,
    scope: ContextId,
    access: Option<LocalEnvironmentAccess>,
}
impl RuntimeHandle {
    pub fn new(services: RuntimeServices, cx: &mut App) -> Self {
        Self {
            runtime: cx.new(|cx| Runtime::new(services, cx)),
            scope: ContextId::General,
            access: Some(LocalEnvironmentAccess::this_computer()),
        }
    }
    /// An immutable view of the same installation service, captured before work starts.
    pub fn for_scope(&self, scope: ContextId) -> Self {
        Self {
            runtime: self.runtime.clone(),
            scope,
            access: self.access.clone(),
        }
    }
    pub fn for_environment(
        &self,
        access: LocalEnvironmentAccess,
    ) -> Result<Self, nocterm_ai::storage::AgentStorageError> {
        access.check_current()?;
        Ok(Self {
            runtime: self.runtime.clone(),
            scope: self.scope,
            access: Some(access),
        })
    }
    pub fn unavailable_account(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            scope: self.scope,
            access: None,
        }
    }
    pub fn environment(
        &self,
    ) -> Result<&LocalEnvironment, nocterm_hubs_model::source_identity::LocalAccountSourceError>
    {
        self.access
            .as_ref()
            .map(LocalEnvironmentAccess::environment)
            .ok_or(nocterm_hubs_model::source_identity::LocalAccountSourceError::Unavailable)
    }
    pub fn chat_scope(&self) -> Result<ChatScope, nocterm_ai::storage::AgentStorageError> {
        Ok(ChatScope::new(self.environment()?.clone(), self.scope))
    }
    fn current_source(
        &self,
    ) -> Result<(), nocterm_hubs_model::source_identity::LocalAccountSourceError> {
        self.access
            .as_ref()
            .ok_or(nocterm_hubs_model::source_identity::LocalAccountSourceError::Unavailable)?
            .check_current()
    }
    pub fn check_current(&self) -> Result<(), nocterm_ai::storage::AgentStorageError> {
        self.current_source().map_err(Into::into)
    }
    /// Retain the original source for native workers that outlive an App callback.
    pub fn captured_source(
        &self,
    ) -> Result<LocalEnvironmentAccess, nocterm_ai::storage::AgentStorageError> {
        self.check_current()?;
        self.access.clone().ok_or_else(|| {
            nocterm_hubs_model::source_identity::LocalAccountSourceError::Unavailable.into()
        })
    }
    /// Native source observations trigger revalidation of retained original leases.
    /// Notifications are hints; each source checks its own actual retained owner.
    pub fn refresh_environment_sources(&self, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.refresh_environment_sources(cx));
    }
    /// Create a chat-owned registry using this installation’s supplied native adapter.
    pub fn command_jobs(&self, cx: &App) -> Box<dyn nocterm_session::command_jobs::CommandJobs> {
        self.runtime.read(cx).services.command_jobs.create()
    }
    /// The transfer port this installation supplies; every chat keeps its own book over it.
    pub fn transfers(&self, cx: &App) -> Arc<dyn nocterm_transfers::TransferRequests> {
        self.runtime.read(cx).services.transfers.clone()
    }
    pub fn scope(&self) -> ContextId {
        self.scope
    }
    pub fn catalog_snapshot(&self, cx: &App) -> Option<crate::CatalogSnapshot> {
        self.check_current().ok()?;
        self.runtime
            .read(cx)
            .catalog_snapshot(self.chat_scope().ok()?)
    }
    pub fn observe(
        &self,
        cx: &mut App,
        mut changed: impl FnMut(&mut App) + 'static,
    ) -> Subscription {
        cx.observe(&self.runtime, move |_, cx| changed(cx))
    }
    /// The installation owns the subscription; the callback does not retain runtime.
    pub fn observe_settings(
        &self,
        settings: &nocterm_store::SettingsHandle,
        cx: &mut App,
    ) -> Subscription {
        let runtime = self.runtime.downgrade();
        settings.observe(cx, move |cx| {
            let _ = runtime.update(cx, |runtime, cx| runtime.settings_changed(cx));
        })
    }
    pub fn favorites<'a>(&self, cx: &'a App) -> &'a nocterm_ai::favorites::AgentStateFile {
        self.runtime.read(cx).favorites(cx)
    }
    pub fn favorites_error(&self, cx: &App) -> Option<AgentPersistenceError> {
        self.runtime.read(cx).favorites_error(cx)
    }
    pub fn history_reader(&self, cx: &App) -> HistoryReader {
        let runtime = self.runtime.read(cx);
        match self.chat_scope() {
            Ok(scope) => runtime
                .scoped_history_reader(scope)
                .with_access(self.access.clone()),
            Err(_) => runtime.chats.unavailable_reader(),
        }
    }
    pub fn limits<'a>(&self, agent: &str, cx: &'a App) -> Option<&'a nocterm_ai::usage::Limits> {
        self.runtime.read(cx).limits(agent)
    }
    pub fn document_is_current(&self, chat_id: &str, owner: EntityId, cx: &App) -> bool {
        if self.check_current().is_err() {
            return false;
        }
        self.runtime.read(cx).scoped_document_is_current(
            &ChatKey::in_environment(
                match self.chat_scope() {
                    Ok(scope) => scope,
                    Err(_) => return false,
                },
                chat_id.to_owned(),
            ),
            owner,
        )
    }
    pub fn terminal_auth_request(
        &self,
        key: u64,
        method: &AuthMethod,
        cx: &App,
    ) -> Option<TerminalAuthRequest> {
        self.check_current().ok()?;
        self.runtime.read(cx).terminal_auth_request(key, method)
    }
    /// The same retained installation budget for Local and remote conversation sources.
    pub fn history_admission(&self, cx: &App) -> crate::history_admission::HistoryAdmissionHandle {
        self.runtime.read(cx).chats.history_admission()
    }
    /// Capture the installation and Runtime borrower identity before Local work.
    pub fn local_history_allocations(
        &self,
        cx: &App,
    ) -> crate::history_admission::LocalHistoryAllocations {
        crate::history_admission::LocalHistoryAllocations::new(
            self.history_admission(cx),
            self.runtime.entity_id(),
        )
    }
    /// Admit before a hydrated body mutates its original registered owner.
    /// All other contexts/windows and immutable writer generations are included.
    pub fn admit_history(
        &self,
        requester: EntityId,
        bytes: usize,
        cx: &mut App,
    ) -> Result<(), nocterm_ai::storage::AgentStorageError> {
        self.check_current()?;
        let residency = self.runtime.read(cx).chats.history_residency(
            self.chat_scope()?,
            Some(requester),
            cx,
        )?;
        residency
            .admit(Some(requester), bytes, cx)
            .map(|_| ())
            .map_err(crate::history_admission::HistoryAdmissionError::into_storage)
    }
    pub fn trim_history(
        &self,
        cx: &mut App,
    ) -> Result<Vec<ChatKey>, nocterm_ai::storage::AgentStorageError> {
        self.check_current()?;
        let residency =
            self.runtime
                .read(cx)
                .chats
                .history_residency(self.chat_scope()?, None, cx)?;
        residency
            .admit(None, 0, cx)
            .map_err(crate::history_admission::HistoryAdmissionError::into_storage)
    }
    /// Admission for a new decoded document is bound to its exact reservation.
    pub fn admit_claimed_history(
        &self,
        claim: &crate::DocumentClaim,
        bytes: usize,
        cx: &mut App,
    ) -> Result<(), crate::ChatOpenError> {
        self.check_current()?;
        nocterm_ai::storage::require_local_scope(&self.chat_scope()?, &claim.key().chat_scope())?;
        let residency = self.runtime.read(cx).chats.claim_residency(claim, cx)?;
        residency
            .admit(None, bytes, cx)
            .map_err(crate::history_admission::HistoryAdmissionError::into_storage)?;
        Ok(())
    }
    pub fn take_prepared_chat(
        &self,
        claim: &crate::DocumentClaim,
        payload: crate::history_admission::HistoryPayloadPermit,
        cx: &mut App,
    ) -> Result<Option<nocterm_core::allocation::Allocated<SavedChat>>, crate::ChatOpenError> {
        self.check_current()?;
        nocterm_ai::storage::require_local_scope(&self.chat_scope()?, &claim.key().chat_scope())?;
        self.runtime.update(cx, |runtime, cx| {
            runtime.take_prepared_chat(claim, payload, cx)
        })
    }
    pub fn toggle_favorite(&self, agent: &str, option: &str, value: &str, cx: &mut App) {
        self.runtime.update(cx, |runtime, cx| {
            runtime.toggle_favorite(agent, option, value, cx)
        })
    }
    pub fn remember_options(&self, agent: &str, options: &[ConfigOption], cx: &mut App) {
        self.runtime.update(cx, |runtime, cx| {
            runtime.remember_options(agent, options, cx)
        })
    }
    pub fn remember_choice(
        &self,
        agent: &str,
        option: &ConfigOption,
        value: &nocterm_ai::session_config::ConfigValue,
        cx: &mut App,
    ) {
        self.runtime.update(cx, |runtime, cx| {
            runtime.remember_choice(agent, option, value, cx)
        })
    }
    pub fn set_last_agent(&self, agent: &str, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.set_last_agent(agent, cx))
    }
    pub fn claim_document(
        &self,
        id: String,
        window: EntityId,
        cx: &mut App,
    ) -> Result<crate::DocumentClaimOutcome, crate::ChatOpenError> {
        self.check_current()?;
        self.runtime.read(cx).claim_local_document(
            ChatKey::in_environment(self.chat_scope()?, id),
            self.access
                .clone()
                .ok_or(nocterm_ai::storage::AgentStorageError::Interrupted)?,
            window,
            None,
        )
    }
    /// Claims chat `id` in `window` in place of `expected_owner`: a restart in the same window,
    /// or a move from another, where the chat must be at rest.
    pub fn replace_document(
        &self,
        id: String,
        expected_owner: EntityId,
        window: EntityId,
        cx: &mut App,
    ) -> Result<crate::DocumentClaimOutcome, crate::ChatOpenError> {
        self.check_current()?;
        if let Some(held) = self.runtime.read(cx).kept_by(expected_owner, window, cx) {
            return Ok(crate::DocumentClaimOutcome::AlreadyOpen { window: held });
        }
        self.runtime.read(cx).claim_local_document(
            ChatKey::in_environment(self.chat_scope()?, id),
            self.access
                .clone()
                .ok_or(nocterm_ai::storage::AgentStorageError::Interrupted)?,
            window,
            Some(expected_owner),
        )
    }
    pub fn commit_document(
        &self,
        claim: crate::DocumentClaim,
        client: Client,
        cx: &mut App,
    ) -> Result<(), crate::ChatOpenError> {
        self.check_current()?;
        nocterm_ai::storage::require_local_scope(&self.chat_scope()?, &claim.key().chat_scope())?;
        self.runtime
            .update(cx, |runtime, cx| runtime.commit_document(claim, client, cx))
    }
    pub fn unregister_document(
        &self,
        id: EntityId,
        cx: &mut App,
    ) -> Result<(), nocterm_ai::storage::AgentStorageError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.unregister_scoped_document(self.chat_scope()?, id, cx)
        })
    }
    pub fn request_activation(
        &self,
        client: Client,
        cx: &mut App,
    ) -> Result<(), crate::AgentActivationError> {
        self.check_current()?;
        self.runtime.update(cx, |runtime, cx| {
            runtime.request_scoped_activation(self.chat_scope()?, client, cx)
        })
    }
    pub fn connect(
        &self,
        client: Client,
        chat_id: String,
        launch: AgentLaunch,
        cx: &mut App,
    ) -> Result<(), crate::AgentActivationError> {
        self.check_current()?;
        self.runtime.update(cx, |runtime, cx| {
            runtime.connect_scoped(self.chat_scope()?, client, chat_id, launch, cx)
        })
    }
    pub fn register_bridge(
        &self,
        client: &Client,
        cx: &mut App,
    ) -> Result<BridgeRegistration, nocterm_ai::connection::BridgeRegistrationError> {
        self.current_source()?;
        self.runtime.update(cx, |runtime, cx| {
            runtime.register_scoped_bridge(
                self.chat_scope().map_err(|_| {
                    nocterm_ai::connection::BridgeRegistrationError::AccountSource(
                        nocterm_hubs_model::source_identity::LocalAccountSourceError::Unavailable,
                    )
                })?,
                client,
                cx,
            )
        })
    }
    pub fn save_chat(
        &self,
        chat: Arc<SharedChat>,
        owner: EntityId,
        revision: u64,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.scoped_save_chat(self.chat_scope()?, chat, owner, revision, cx)
        })
    }
    pub fn capture_and_detach_document(
        &self,
        id: EntityId,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.capture_scoped_document(self.chat_scope()?, id, cx)
        })
    }
    /// The chats of this collection windows other than `window` hold, which `window` lists
    /// without holding them.
    pub fn held_elsewhere(&self, window: EntityId, cx: &App) -> Vec<gpui::AnyWeakEntity> {
        let (Ok(()), Ok(scope)) = (self.check_current(), self.chat_scope()) else {
            return Vec::new();
        };
        self.runtime
            .read(cx)
            .held_elsewhere(&scope, window)
            .iter()
            .filter_map(|client| client.chat())
            .collect()
    }
    /// Asks the window that holds chat `chat` to show it and come forward.
    pub fn reveal_chat(&self, chat: EntityId, cx: &mut App) {
        self.runtime
            .update(cx, |_, cx| cx.emit(crate::HolderEvent::Reveal { chat }));
    }
    /// Whether chat `chat`, which another window holds, may move to `window`: it neither works
    /// nor shows in its own.
    pub fn may_move(&self, chat: EntityId, window: EntityId, cx: &App) -> bool {
        let runtime = self.runtime.read(cx);
        runtime.chats.window_of(chat).is_some() && runtime.kept_by(chat, window, cx).is_none()
    }
    /// Calls `event` for what the runtime tells every window about the chats windows hold.
    pub fn observe_holders(
        &self,
        cx: &mut App,
        mut event: impl FnMut(&crate::HolderEvent, &mut App) + 'static,
    ) -> Subscription {
        cx.subscribe(&self.runtime, move |_, notice, cx| event(notice, cx))
    }
    pub fn delete_chat(&self, id: String, cx: &mut App) {
        if self.check_current().is_err() {
            return;
        }
        let Ok(scope) = self.chat_scope() else {
            return;
        };
        self.runtime.update(cx, |runtime, cx| {
            runtime.scoped_delete_chat(ChatKey::in_environment(scope, id), cx)
        })
    }
    /// Request the captured context once; an existing load or snapshot is retained.
    pub fn ensure_history(&self, cx: &mut App) -> Result<(), crate::CatalogLoadError> {
        self.current_source()?;
        self.runtime.update(cx, |runtime, cx| {
            runtime.chats.ensure(self.chat_scope()?, cx)
        })
    }
    pub fn try_reload_history(&self, cx: &mut App) -> Result<(), crate::CatalogLoadError> {
        self.current_source()?;
        self.runtime.update(cx, |runtime, cx| {
            runtime.try_scoped_reload_history(self.chat_scope()?, cx)
        })
    }
    pub fn reload_history(&self, cx: &mut App) {
        if self.check_current().is_err() {
            return;
        }
        let Ok(scope) = self.chat_scope() else {
            return;
        };
        self.runtime
            .update(cx, |runtime, cx| runtime.scoped_reload_history(scope, cx))
    }
    pub fn settings_changed(&self, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.settings_changed(cx))
    }
    pub fn capture_shutdown(
        &self,
        cx: &mut App,
    ) -> BoxFuture<'static, Result<(), nocterm_store::StoreError>> {
        self.runtime
            .update(cx, |runtime, cx| runtime.capture_shutdown(cx))
    }
    pub fn shutdown(&self, cx: &mut App) -> Task<()> {
        self.runtime.update(cx, |runtime, cx| runtime.shutdown(cx))
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn inject_events(
        &self,
        events: async_channel::Receiver<nocterm_ai::AgentEvent>,
        cx: &mut App,
    ) {
        self.runtime
            .update(cx, |runtime, cx| runtime.inject_events(events, cx))
    }
}
