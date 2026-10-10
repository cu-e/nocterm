//! Only durable chat state transfers to a replacement process.
use super::{AgentThread, Composer, PromptMetadata, Restore};
use nocterm_ai::thread::ThreadState;
pub(crate) struct RestartData {
    pub agent: String,
    chat_id: String,
    updated: u64,
    name: Option<String>,
    pinned: bool,
    history_queued: bool,
    state: ThreadState,
    last_prompt: Option<PromptMetadata>,
    last_model: Option<String>,
    composer: Composer,
    fallback_history: bool,
    restore: Option<Restore>,
    config_choices: nocterm_ai::session_config::Choices,
    mode_choice: Option<nocterm_ai::acp::SessionModeId>,
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
            last_prompt: self.last_prompt.clone(),
            last_model: self.last_model.clone(),
            composer: self.composer.restarted(),
            fallback_history: self.fallback_history,
            restore: self.restore_descriptor(),
            config_choices: self.config_choices.clone(),
            mode_choice: self.mode_choice.clone(),
        }
    }
    pub(crate) fn apply_restart(&mut self, data: RestartData) {
        self.chat_id = data.chat_id;
        self.updated = data.updated;
        self.name = data.name;
        self.pinned = data.pinned;
        self.history_queued = data.history_queued;
        self.state = data.state;
        self.last_prompt = data.last_prompt;
        self.last_model = data.last_model;
        self.composer = data.composer;
        self.fallback_history = data.fallback_history;
        self.restore = data.restore;
        self.config_choices = data.config_choices;
        self.mode_choice = data.mode_choice;
    }
}
