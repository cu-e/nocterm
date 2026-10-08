//! Prompt snapshots and FIFO dispatch. Cancellation completes before dispatch resumes.
use gpui_kit::{App, Context};
use nocterm_ai::history::{SavedAttachment, SavedPrompt};
use nocterm_ui::ActiveAi as _;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::{AgentThread, Attachment};

#[derive(Clone)]
pub(crate) struct QueuedPrompt {
    pub saved: Arc<SavedPrompt>,
    pub image_keys: Arc<Vec<Option<(usize, usize, bool)>>>,
    pub attachments: Vec<Attachment>,
    pub encoded_len: usize,
}

impl QueuedPrompt {
    #[cfg(test)]
    pub(crate) fn new(saved: SavedPrompt, attachments: Vec<Attachment>) -> Self {
        Self::prepared(saved, attachments, None)
    }
    pub(crate) fn prepared(
        saved: SavedPrompt,
        attachments: Vec<Attachment>,
        encoded_len: Option<usize>,
    ) -> Self {
        static NEXT_IMAGE: AtomicUsize = AtomicUsize::new(1);
        let encoded_len = encoded_len
            .unwrap_or_else(|| nocterm_ai::history::prompt_size(&saved).unwrap_or(usize::MAX));
        let image_keys = saved
            .images
            .iter()
            .map(|block| match block {
                nocterm_ai::acp::ContentBlock::Image(image) => Some((
                    NEXT_IMAGE.fetch_add(1, Ordering::Relaxed),
                    image.data.len(),
                    true,
                )),
                _ => None,
            })
            .collect();
        Self {
            saved: Arc::new(saved),
            attachments,
            image_keys: Arc::new(image_keys),
            encoded_len,
        }
    }
}

impl AgentThread {
    pub(crate) fn stable_attachments(
        &self,
        values: &[Attachment],
        cx: &App,
    ) -> Vec<SavedAttachment> {
        let terminals = self
            .workspace
            .upgrade()
            .map(|workspace| workspace.read(cx).terminals(cx))
            .unwrap_or_default();
        let mut saved = Vec::new();
        for attachment in values {
            let value = match attachment {
                Attachment::Connection(id) => Some(SavedAttachment::Connection(id.clone())),
                Attachment::Group(id) => Some(SavedAttachment::Group(id.clone())),
                Attachment::UnavailableLocal(title) => {
                    Some(SavedAttachment::LocalTerminal(title.clone()))
                }
                Attachment::Terminal(id) => terminals
                    .iter()
                    .find(|entry| entry.item == *id)
                    .and_then(|entry| entry.access.info(cx))
                    .map(|info| match info.profile {
                        Some(id) => SavedAttachment::Connection(id.to_string()),
                        None => SavedAttachment::LocalTerminal(info.title.to_string()),
                    }),
            };
            if let Some(value) = value
                && !saved.contains(&value)
            {
                saved.push(value);
            }
        }
        saved
    }

    pub(crate) fn submit(
        &mut self,
        text: String,
        replace: Option<u64>,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        if !cx.ai_enabled() || self.auth_required || self.ended() {
            return Ok(false);
        }
        if text.trim().is_empty() && self.images.is_empty() {
            return Ok(false);
        }
        let id = replace.unwrap_or(self.next_queue_id);
        let saved = SavedPrompt {
            id,
            text,
            images: Vec::new(),
            attachments: self.stable_attachments(&self.attachments, cx),
        };
        let mut saved = saved;
        let encoded_len = nocterm_ai::history::prompt_size(&saved)?
            + self
                .images
                .iter()
                .map(|image| image.encoded_len())
                .sum::<usize>()
            + self.images.len().saturating_sub(1);
        saved.images = self.images.iter().map(|image| image.content()).collect();
        let prompt = QueuedPrompt::prepared(saved, self.attachments.clone(), Some(encoded_len));
        let mut queue = self.queue.clone();
        if let Some(id) = replace {
            let Some(slot) = queue.iter_mut().find(|prompt| prompt.saved.id == id) else {
                return Err("This message was already sent.".into());
            };
            *slot = prompt;
        } else {
            queue.push(prompt);
        }
        self.refresh_storage_budget()?;
        let mut metadata = self.history_metadata(cx);
        // Sending moves ordinary text into the queue: budget it once.
        if replace.is_none() {
            metadata.draft = None;
        }
        self.storage_budget
            .validate(&metadata, queue.iter().map(|prompt| prompt.encoded_len))?;
        self.queue = queue;
        if replace.is_none() {
            self.draft = None;
            self.draft_save = None;
            self.draft_changed = false;
        }
        if replace.is_none() {
            self.next_queue_id += 1;
        }
        self.images.clear();
        self.persist(cx);
        if !self.generating && replace.is_none() {
            self.queue_paused = false;
            self.dispatch_next(cx);
        }
        cx.notify();
        Ok(true)
    }

    pub(crate) fn dispatch_next(&mut self, cx: &mut Context<Self>) {
        if self.queue_paused
            || self.queue_editing
            || self.generating
            || self.queue.is_empty()
            || self.auth_required
            || self.ended()
        {
            return;
        }
        if self.persistence_error.is_some()
            || (self.lease.is_none() && self.persisted_revision < self.document_revision)
        {
            return;
        }
        if self.session().is_none() {
            self.request_activation(cx);
            return;
        }
        let prompt = self.queue.remove(0);
        // Context is fixed for this turn, including terminal tool access.
        self.prompt_attachments = Some(prompt.attachments);
        let saved = Arc::unwrap_or_clone(prompt.saved);
        self.start_prompt(saved.text, saved.images, cx);
    }

    pub(crate) fn send_now(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.queue.iter().position(|prompt| prompt.saved.id == id) else {
            return;
        };
        let prompt = self.queue.remove(index);
        self.queue.insert(0, prompt);
        if self.generating {
            self.stop(cx);
        }
        self.queue_paused = false;
        self.persist(cx);
        // When generating, only the old prompt's completion may dispatch.
        if !self.generating {
            self.dispatch_next(cx);
        }
        cx.notify();
    }
}

pub(crate) fn restore_attachments(values: &[SavedAttachment]) -> Vec<Attachment> {
    values
        .iter()
        .map(|value| match value {
            SavedAttachment::Connection(id) => Attachment::Connection(id.clone()),
            SavedAttachment::Group(id) => Attachment::Group(id.clone()),
            SavedAttachment::LocalTerminal(title) => Attachment::UnavailableLocal(title.clone()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rendering_snapshot_shares_image_payload_and_precomputed_keys() {
        let prompt = QueuedPrompt::new(
            SavedPrompt {
                id: 1,
                text: "image".into(),
                images: vec![nocterm_ai::acp::ContentBlock::Image(
                    nocterm_ai::acp::ImageContent::new("a".repeat(2 * 1024 * 1024), "image/png"),
                )],
                attachments: Vec::new(),
            },
            Vec::new(),
        );
        let copy = prompt.clone();
        assert!(Arc::ptr_eq(&prompt.saved, &copy.saved));
        assert!(Arc::ptr_eq(&prompt.image_keys, &copy.image_keys));
        assert_eq!(copy.image_keys[0].unwrap().1, 2 * 1024 * 1024);
    }
}
