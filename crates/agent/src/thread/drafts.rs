//! Literal accepted edits advance semantic state immediately; only admission is debounced.
use super::AgentThread;
use gpui_kit::{App, Context};
use std::{sync::Arc, time::Duration};
impl AgentThread {
    pub(crate) fn set_draft(&mut self, text: String, cx: &mut Context<Self>) {
        if self.is_archived() {
            let now = nocterm_ai::time::now();
            if self.archive.edit_draft(text, now) {
                self.state.edited_archived_draft(now);
                cx.notify();
            }
            return;
        }
        if !self.composer.set_draft(text) {
            return;
        }
        self.accept_document_inputs(cx, true);
        if !self.state.has_debounce() {
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let _ = this.update(cx, |this, cx| this.save(cx));
            });
            self.state.replace_debounce(task);
        }
        cx.notify();
    }
    pub(crate) fn flush_draft(&mut self, cx: &mut Context<Self>) {
        if self.state.draft_changed() {
            self.save(cx);
        }
    }
    pub(crate) fn flush_snapshot(&self, cx: &App) -> Option<Arc<nocterm_ai::history::SharedChat>> {
        if self.state.clean() {
            return None;
        }
        if let Some(snapshot) = self.state.current_snapshot() {
            return Some(snapshot);
        }
        if self.is_archived() {
            return self
                .archive
                .draft_patch(self.state.chat_id(), self.state.agent_id())
                .map(|metadata| {
                    Arc::new(nocterm_ai::history::SharedChat {
                        metadata,
                        queue: Vec::new(),
                    })
                });
        }
        self.shared_snapshot(cx).or_else(|| {
            self.state.history_present().then(|| {
                Arc::new(nocterm_ai::history::SharedChat {
                    metadata: self.history_metadata(cx),
                    queue: Vec::new(),
                })
            })
        })
    }
    pub(crate) fn capture_pending(
        &mut self,
        cx: &App,
    ) -> Option<(Arc<nocterm_ai::history::SharedChat>, u64)> {
        self.state.cancel_debounce();
        if !self.state.needs_candidate() {
            return self.state.existing_offer();
        }
        if self.is_archived() {
            let snapshot = self
                .archive
                .draft_patch(self.state.chat_id(), self.state.agent_id())
                .map(|metadata| {
                    Arc::new(nocterm_ai::history::SharedChat {
                        metadata,
                        queue: Vec::new(),
                    })
                })?;
            return self.state.capture_patch(snapshot);
        }
        let attachments = self.stable_attachments(self.composer.default_attachments(), cx);
        let resume = self.persistent_resume();
        let input = super::document::Inputs::new(&self.composer, &self.config, attachments, resume);
        self.state.propose_candidate(&input)
    }
}
