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
    history::{ChatSummary, SavedChat, SharedChat},
};
use std::sync::Arc;

#[derive(Clone)]
pub struct RuntimeHandle {
    runtime: Entity<Runtime>,
}
impl RuntimeHandle {
    pub fn new(services: RuntimeServices, cx: &mut App) -> Self {
        Self {
            runtime: cx.new(|cx| Runtime::new(services, cx)),
        }
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
        self.runtime.read(cx).history_reader()
    }
    pub fn limits<'a>(&self, agent: &str, cx: &'a App) -> Option<&'a nocterm_ai::usage::Limits> {
        self.runtime.read(cx).limits(agent)
    }
    pub fn document_is_current(&self, chat_id: &str, owner: EntityId, cx: &App) -> bool {
        self.runtime.read(cx).document_is_current(chat_id, owner)
    }
    pub fn terminal_auth_request(
        &self,
        key: u64,
        method: &AuthMethod,
        cx: &App,
    ) -> Option<TerminalAuthRequest> {
        self.runtime.read(cx).terminal_auth_request(key, method)
    }
    pub fn take_history(&self, cx: &mut App) -> (Option<Vec<SavedChat>>, Option<Vec<ChatSummary>>) {
        self.runtime.update(cx, |runtime, _| {
            (runtime.take_saved_chats(), runtime.take_saved_catalog())
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
    pub fn refresh_limits(&self, agent: &str, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.refresh_limits(agent, cx))
    }
    pub fn register_document(&self, client: Client, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.register_document(client, cx))
    }
    pub fn unregister_document(&self, id: EntityId, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.unregister_document(id, cx))
    }
    pub fn request_activation(&self, client: Client, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.request_activation(client, cx))
    }
    pub fn connect(&self, client: Client, chat_id: String, launch: AgentLaunch, cx: &mut App) {
        self.runtime.update(cx, |runtime, cx| {
            runtime.connect(client, chat_id, launch, cx)
        })
    }
    pub fn register_bridge(
        &self,
        client: &Client,
        cx: &mut App,
    ) -> Result<BridgeRegistration, nocterm_ai::connection::BridgeRegistrationError> {
        self.runtime
            .update(cx, |runtime, cx| runtime.register_bridge(client, cx))
    }
    pub fn save_chat(
        &self,
        chat: Arc<SharedChat>,
        owner: EntityId,
        revision: u64,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.save_chat(chat, owner, revision, cx)
        })
    }
    pub fn capture_and_detach_document(
        &self,
        id: EntityId,
        cx: &mut App,
    ) -> Result<bool, AgentPersistenceError> {
        self.runtime.update(cx, |runtime, cx| {
            runtime.capture_and_detach_document(id, cx)
        })
    }
    pub fn delete_chat(&self, id: String, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.delete_chat(id, cx))
    }
    pub fn reload_history(&self, cx: &mut App) {
        self.runtime
            .update(cx, |runtime, cx| runtime.reload_history(cx))
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
