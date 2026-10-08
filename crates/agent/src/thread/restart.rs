//! Only durable chat state transfers to a replacement process.
use super::{AgentThread, Attachment, PromptMetadata, Restore, queue};
use nocterm_ai::thread::ThreadState;
pub(crate) struct RestartData {
    pub agent: String,
    chat_id: String,
    updated: u64,
    name: Option<String>,
    pinned: bool,
    draft: Option<String>,
    history_queued: bool,
    state: ThreadState,
    attachments: Vec<Attachment>,
    images: Vec<nocterm_ai::images::PromptImage>,
    last_prompt: Option<PromptMetadata>,
    last_model: Option<String>,
    queue: Vec<queue::QueuedPrompt>,
    next_queue_id: u64,
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
            draft: self.draft.clone(),
            history_queued: self.history_queued,
            state: self.state.clone(),
            attachments: self.default_attachments().to_vec(),
            images: self.images.clone(),
            last_prompt: self.last_prompt.clone(),
            last_model: self.last_model.clone(),
            queue: self.queue.clone(),
            next_queue_id: self.next_queue_id,
            fallback_history: self.fallback_history,
            restore: self.restore_descriptor(),
        }
    }
    pub(crate) fn apply_restart(&mut self, data: RestartData) {
        self.chat_id = data.chat_id;
        self.updated = data.updated;
        self.name = data.name;
        self.pinned = data.pinned;
        self.draft = data.draft;
        self.history_queued = data.history_queued;
        self.state = data.state;
        self.attachments = data.attachments;
        self.images = data.images;
        self.last_prompt = data.last_prompt;
        self.last_model = data.last_model;
        self.queue = data.queue;
        self.next_queue_id = data.next_queue_id;
        self.queue_paused = true;
        self.fallback_history = data.fallback_history;
        self.restore = data.restore;
    }
}
