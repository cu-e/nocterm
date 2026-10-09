//! Literal unsent text is model-owned; only snapshots are debounced.
use super::AgentThread;
use gpui_kit::{App, Context};
use std::{sync::Arc, time::Duration};

impl AgentThread {
    pub(crate) fn set_draft(&mut self, text: String, cx: &mut Context<Self>) {
        if self.archive.is_some() {
            self.pending_archive_draft = Some(text);
            self.updated = nocterm_ai::time::now();
            return;
        }
        if !self.composer.set_draft(text) {
            return;
        }
        self.draft_changed = true;
        self.updated = nocterm_ai::time::now();
        if self.draft_save.is_none() {
            self.draft_save = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let _ = this.update(cx, |this, cx| this.save(cx));
            }));
        }
        cx.notify();
    }

    pub(crate) fn flush_draft(&mut self, cx: &mut Context<Self>) {
        if self.draft_changed {
            self.save(cx);
        }
    }

    pub(crate) fn flush_snapshot(&self, cx: &App) -> Option<Arc<nocterm_ai::history::SharedChat>> {
        if self.archive.is_some() {
            return self.pending_archive_draft.as_ref().map(|text| {
                Arc::new(nocterm_ai::history::SharedChat {
                    metadata: nocterm_ai::history::SavedChat::archive_draft(
                        self.chat_id.clone(),
                        self.agent_id.clone(),
                        text.clone(),
                        self.updated,
                    ),
                    queue: Vec::new(),
                })
            });
        }
        self.shared_snapshot(cx).or_else(|| {
            (self.history_queued || self.draft_changed).then(|| {
                Arc::new(nocterm_ai::history::SharedChat {
                    metadata: self.history_metadata(cx),
                    queue: Vec::new(),
                })
            })
        })
    }

    /// An empty snapshot clears an earlier draft without permanently deleting
    /// its chat id. Truly empty chats are omitted when history is read back.
    pub(crate) fn pending_snapshot(
        &self,
        cx: &App,
    ) -> Option<Arc<nocterm_ai::history::SharedChat>> {
        if self.archive.is_some() {
            return None;
        }
        self.shared_snapshot(cx).or_else(|| {
            self.draft_changed.then(|| {
                Arc::new(nocterm_ai::history::SharedChat {
                    metadata: self.history_metadata(cx),
                    queue: Vec::new(),
                })
            })
        })
    }
}
