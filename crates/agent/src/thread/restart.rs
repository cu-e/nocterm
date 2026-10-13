//! Replacement processes transfer durable meaning without allocating a new chat identity.
use super::{AgentThread, Composer, Restore};
use gpui_kit::Context;
pub(crate) struct RestartData {
    document: super::document::DocumentRestart,
    config: super::config::RestartConfig,
    composer: Composer,
    restore: Option<Restore>,
}
impl RestartData {
    pub(crate) fn agent_id(&self) -> &str {
        self.document.agent_id()
    }
}
impl AgentThread {
    pub(crate) fn restart_data(&self) -> RestartData {
        RestartData {
            document: self.state.restart_snapshot(),
            config: self.config.restart_snapshot(),
            composer: self.composer.restarted(),
            restore: self.restore_descriptor(),
        }
    }
    pub(crate) fn restarted(
        data: RestartData,
        host: crate::chat_host::ChatHost,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = super::document::ChatDocument::restarted(data.document);
        let mut thread = Self::from_document(state, host, cx);
        thread.config.hydrate_restart(data.config);
        thread.composer = data.composer;
        thread.restore = data.restore;
        thread
    }
}
