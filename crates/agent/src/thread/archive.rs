//! Archived rows hold metadata only until an action needs the document.
use super::AgentThread;
use crate::runtime::Runtime;
use gpui_kit::Context;
use nocterm_ai::history::ChatSummary;
use nocterm_ui::ActiveAi as _;

pub(super) type Loaded = Box<dyn FnOnce(&mut AgentThread, &mut Context<AgentThread>)>;
impl AgentThread {
    pub(crate) fn archived(
        summary: ChatSummary,
        workspace: gpui_kit::WeakEntity<nocterm_workspace::Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut thread = Self::new(summary.agent_id.clone(), workspace, cx);
        thread.chat_id = summary.id.clone();
        thread.name = summary.name.clone();
        thread.pinned = summary.pinned;
        thread.updated = summary.updated;
        thread.last_model = summary.model.clone();
        thread.status = "Saved chat".into();
        thread.archive = Some(summary);
        thread
    }
    pub(crate) fn load_archive(
        &mut self,
        cx: &mut Context<Self>,
        loaded: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        if self.archive.is_none() {
            loaded(self, cx);
            return;
        }
        self.archive_actions.push(Box::new(loaded));
        if self.loading_archive {
            return;
        }
        self.loading_archive = true;
        self.status = "Loading saved chat…".into();
        let id = self.chat_id.clone();
        let epoch = self.epoch;
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        let gate = Runtime::global(cx).read(cx).history_gate.clone();
        let loading = cx.background_executor().spawn(async move {
            let permit = gate.acquire().await;
            (nocterm_ai::history::load(&dir, &id), permit)
        });
        cx.spawn(async move |this, cx| {
            let (result, permit) = loading.await;
            let _ = this.update(cx, |this, cx| {
                if this.epoch != epoch
                    || !cx.ai_enabled()
                    || this.archive.is_none()
                    || !Runtime::global(cx)
                        .read(cx)
                        .document_is_current(&this.chat_id, cx.entity_id())
                {
                    return;
                }
                this.loading_archive = false;
                this.archive_permit = Some(permit);
                match result {
                    Ok(chat) => {
                        this.restore_content(chat);
                        if this.draft_changed {
                            this.save(cx);
                        }
                        for action in std::mem::take(&mut this.archive_actions) {
                            action(this, cx);
                        }
                    }
                    Err(error) => {
                        this.archive_actions.clear();
                        this.status = format!("Could not load saved chat: {error}");
                        this.status_error = true;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn resident_history_bytes(&self) -> usize {
        if self.archive.is_some() {
            return 0;
        }
        self.resident_archive_bytes
            .max(self.storage_budget.resident_bytes())
            + self
                .composer
                .queue
                .iter()
                .map(|prompt| prompt.memory_len)
                .sum::<usize>()
            + self.composer.draft.as_ref().map_or(0, String::len)
    }
    pub(crate) fn evict_history(&mut self, cx: &Context<Self>) -> bool {
        if self.archive.is_some()
            || self.lease.is_some()
            || self.session_busy()
            || self.document_revision != self.persisted_revision
            || self.draft_changed
            || self.persistence_error.is_some()
            || !self.history_queued
        {
            return false;
        }
        let mut summary = ChatSummary::from_chat(&self.history_metadata(cx));
        summary.title = Some(self.title());
        summary.has_content = !self.state.entries.is_empty() || !self.composer.is_empty();
        self.archive = Some(summary);
        self.state = Default::default();
        self.composer = Default::default();
        self.storage_budget = Default::default();
        self.resident_archive_bytes = 0;
        self.dirty_rows.clear();
        self.storage_dirty.clear();
        self.tool_displays = Default::default();
        true
    }
}
