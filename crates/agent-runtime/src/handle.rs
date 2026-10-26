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
};
use std::sync::Arc;

#[derive(Clone)]
pub struct RuntimeHandle {
    runtime: Entity<Runtime>,
    scope: ContextId,
}
impl RuntimeHandle {
    pub fn new(services: RuntimeServices, cx: &mut App) -> Self {
        Self {
            runtime: cx.new(|cx| Runtime::new(services, cx)),
            scope: ContextId::General,
        }
    }
    /// An immutable view of the same installation service, captured before work starts.
    pub fn for_scope(&self, scope: ContextId) -> Self {
        Self {
            runtime: self.runtime.clone(),
            scope,
        }
    }
    pub fn scope(&self) -> ContextId {
        self.scope
    }
    pub fn catalog_snapshot(&self, cx: &App) -> Option<crate::CatalogSnapshot> {
        self.runtime.read(cx).catalog_snapshot(self.scope)
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
        self.runtime.read(cx).scoped_history_reader(self.scope)
    }
    pub fn limits<'a>(&self, agent: &str, cx: &'a App) -> Option<&'a nocterm_ai::usage::Limits> {
        self.runtime.read(cx).limits(agent)
    }
    pub fn document_is_current(&self, chat_id: &str, owner: EntityId, cx: &App) -> bool {
        self.runtime
            .read(cx)
            .scoped_document_is_current(&ChatKey::new(self.scope, chat_id.to_owned()), owner)
    }
    pub fn terminal_auth_request(
        &self,
        key: u64,
        method: &AuthMethod,
        cx: &App,
    ) -> Option<TerminalAuthRequest> {
        self.runtime.read(cx).terminal_auth_request(key, method)
    }
    /// Admit before a hydrated body mutates its original registered owner.
    /// All other contexts/windows and immutable writer generations are included.
    pub fn admit_history(
        &self,
        requester: EntityId,
        bytes: usize,
        cx: &mut App,
    ) -> Result<(), nocterm_ai::storage::AgentStorageError> {
        let residency =
            self.runtime
                .read(cx)
                .chats
                .history_residency(self.scope, Some(requester), cx)?;
        residency.admit(Some(requester), bytes, cx).map(|_| ())
    }
    pub fn trim_history(
        &self,
        cx: &mut App,
    ) -> Result<Vec<ChatKey>, nocterm_ai::storage::AgentStorageError> {
        let residency = self
            .runtime
            .read(cx)
            .chats
            .history_residency(self.scope, None, cx)?;
        residency.admit(None, 0, cx)
    }
    /// Admission for a new decoded document is bound to its exact reservation.
    pub fn admit_claimed_history(
        &self,
        claim: &crate::DocumentClaim,
        bytes: usize,
        cx: &mut App,
    ) -> Result<(), crate::ChatOpenError> {
        nocterm_ai::storage::require_chat_scope(self.scope, claim.key().scope())?;
        let residency = self.runtime.read(cx).chats.claim_residency(claim, cx)?;
        residency.admit(None, bytes, cx)?;
        Ok(())
    }
    pub fn take_prepared_chat(
        &self,
        claim: &crate::DocumentClaim,
        cx: &mut App,
    ) -> Result<Option<SavedChat>, crate::ChatOpenError> {
        nocterm_ai::storage::require_chat_scope(self.scope, claim.key().scope())?;
        self.runtime
            .update(cx, |runtime, _| runtime.take_prepared_chat(claim))
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
    pub fn refresh_limits(&self, agent: &str, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.refresh_limits(agent, cx))
    }
    pub fn claim_document(
        &self,
        id: String,
        window: EntityId,
        cx: &mut App,
    ) -> Result<crate::DocumentClaimOutcome, crate::ChatOpenError> {
        self.runtime
            .read(cx)
            .claim_document(ChatKey::new(self.scope, id), window, None)
    }
    pub fn replace_document(
        &self,
        id: String,
        expected_owner: EntityId,
        window: EntityId,
        cx: &mut App,
    ) -> Result<crate::DocumentClaimOutcome, crate::ChatOpenError> {
        self.runtime.read(cx).claim_document(
            ChatKey::new(self.scope, id),
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
        nocterm_ai::storage::require_chat_scope(self.scope, claim.key().scope())?;
        self.runtime
            .update(cx, |runtime, cx| runtime.commit_document(claim, client, cx))
    }
    pub fn unregister_document(
        &self,
        id: EntityId,
        cx: &mut App,
    ) -> Result<(), nocterm_ai::storage::AgentStorageError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.unregister_scoped_document(self.scope, id, cx)
        })
    }
    pub fn request_activation(
        &self,
        client: Client,
        cx: &mut App,
    ) -> Result<(), crate::AgentActivationError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.request_scoped_activation(self.scope, client, cx)
        })
    }
    pub fn connect(
        &self,
        client: Client,
        chat_id: String,
        launch: AgentLaunch,
        cx: &mut App,
    ) -> Result<(), crate::AgentActivationError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.connect_scoped(self.scope, client, chat_id, launch, cx)
        })
    }
    pub fn register_bridge(
        &self,
        client: &Client,
        cx: &mut App,
    ) -> Result<BridgeRegistration, nocterm_ai::connection::BridgeRegistrationError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.register_scoped_bridge(self.scope, client, cx)
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
            runtime.scoped_save_chat(self.scope, chat, owner, revision, cx)
        })
    }
    pub fn capture_and_detach_document(
        &self,
        id: EntityId,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.capture_scoped_document(self.scope, id, cx)
        })
    }
    pub fn delete_chat(&self, id: String, cx: &mut App) {
        self.runtime.update(cx, |runtime, cx| {
            runtime.scoped_delete_chat(ChatKey::new(self.scope, id), cx)
        })
    }
    /// Request the captured context once; an existing load or snapshot is retained.
    pub fn ensure_history(&self, cx: &mut App) -> Result<(), crate::CatalogLoadError> {
        self.runtime
            .update(cx, |runtime, cx| runtime.chats.ensure(self.scope, cx))
    }
    pub fn try_reload_history(&self, cx: &mut App) -> Result<(), crate::CatalogLoadError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.try_scoped_reload_history(self.scope, cx)
        })
    }
    pub fn reload_history(&self, cx: &mut App) {
        self.runtime.update(cx, |runtime, cx| {
            runtime.scoped_reload_history(self.scope, cx)
        })
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
