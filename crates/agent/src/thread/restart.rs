//! Only durable chat state transfers to a replacement process.
use super::{AgentThread, Composer, Restore};
use nocterm_ai::thread::ThreadState;
pub(crate) struct RestartData {
    pub agent: String,
    chat_id: String,
    updated: u64,
    name: Option<String>,
    pinned: bool,
    history_queued: bool,
    state: ThreadState,
    config: super::config::RestartConfig,
    composer: Composer,
    fallback_history: bool,
    restore: Option<Restore>,
}
impl AgentThread {
    pub(crate) fn restart_data(&self) -> RestartData {
        RestartData {
            agent: self.agent_id.clone(),
            chat_id: self.chat_id.clone(),
            updated: self.updated,
            name: self.name.clone(),
            pinned: self.pinned,
            history_queued: self.history_queued,
            state: self.state.clone(),
            config: self.config.restart_snapshot(),
            composer: self.composer.restarted(),
            fallback_history: self.fallback_history,
            restore: self.restore_descriptor(),
        }
    }
    pub(crate) fn apply_restart(&mut self, data: RestartData) {
        self.chat_id = data.chat_id;
        self.updated = data.updated;
        self.name = data.name;
        self.pinned = data.pinned;
        self.history_queued = data.history_queued;
        self.state = data.state;
        self.config.hydrate_restart(data.config);
        self.composer = data.composer;
        self.fallback_history = data.fallback_history;
        self.restore = data.restore;
    }
}
