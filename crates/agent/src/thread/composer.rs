//! What the user is composing, apart from the conversation it goes into: the
//! unsent draft, the context and images for the next prompt, and the queue of
//! prompts waiting their turn.
use super::{Attachment, queue::QueuedPrompt};
use nocterm_ai::images::PromptImage;

pub(crate) struct Composer {
    pub draft: Option<String>,
    pub attachments: Vec<Attachment>,
    pub images: Vec<PromptImage>,
    pub queue: Vec<QueuedPrompt>,
    pub queue_paused: bool,
    /// Composer edits suspend dispatch independently of stops and errors.
    pub queue_editing: bool,
    /// The context and images to return to when editing a queued prompt ends.
    defaults: Option<Defaults>,
    next_queue_id: u64,
}

struct Defaults {
    attachments: Vec<Attachment>,
    images: Vec<PromptImage>,
}

impl Default for Composer {
    fn default() -> Self {
        Self {
            draft: None,
            attachments: Vec::new(),
            images: Vec::new(),
            queue: Vec::new(),
            queue_paused: false,
            queue_editing: false,
            defaults: None,
            next_queue_id: 1,
        }
    }
}

impl Composer {
    /// A saved chat's composer. Its queue waits until the user resumes it.
    pub(crate) fn restored(
        draft: Option<String>,
        attachments: Vec<Attachment>,
        queue: Vec<QueuedPrompt>,
    ) -> Self {
        let next_queue_id = queue
            .iter()
            .map(|prompt| prompt.saved.id)
            .max()
            .unwrap_or(0)
            + 1;
        Self {
            draft: draft.filter(|draft| !draft.is_empty()),
            attachments,
            queue,
            queue_paused: true,
            next_queue_id,
            ..Self::default()
        }
    }

    /// The state a replacement agent process starts from: paused, and out of
    /// any edit of a queued prompt.
    pub(crate) fn restarted(&self) -> Self {
        Self {
            draft: self.draft.clone(),
            attachments: self.default_attachments().to_vec(),
            images: self.images.clone(),
            queue: self.queue.clone(),
            queue_paused: true,
            next_queue_id: self.next_queue_id,
            ..Self::default()
        }
    }

    /// Nothing typed and nothing waiting.
    pub(crate) fn is_empty(&self) -> bool {
        self.draft.is_none() && self.queue.is_empty()
    }

    /// Sets the draft; an empty text clears it. Returns whether it changed.
    pub(crate) fn set_draft(&mut self, text: String) -> bool {
        let draft = (!text.is_empty()).then_some(text);
        if self.draft == draft {
            return false;
        }
        self.draft = draft;
        true
    }

    /// The id the next new queued prompt takes.
    pub(crate) fn next_queue_id(&self) -> u64 {
        self.next_queue_id
    }

    /// The queue changed and its prompts wait for dispatch.
    pub(crate) fn accept(&mut self, queue: Vec<QueuedPrompt>, replaced: bool) {
        self.queue = queue;
        if !replaced {
            self.draft = None;
            self.next_queue_id += 1;
        }
        self.images.clear();
    }

    /// Queued prompts wait to be sent and nothing holds them back.
    pub(crate) fn dispatchable(&self) -> bool {
        !self.queue.is_empty() && !self.queue_paused && !self.queue_editing
    }

    /// Takes the next prompt to send, when dispatch is not held back.
    pub(crate) fn take_next(&mut self) -> Option<QueuedPrompt> {
        self.dispatchable().then(|| self.queue.remove(0))
    }

    /// Moves the queued prompt `id` to the front. Returns whether it is queued.
    pub(crate) fn move_to_front(&mut self, id: u64) -> bool {
        let Some(index) = self.queue.iter().position(|prompt| prompt.saved.id == id) else {
            return false;
        };
        let prompt = self.queue.remove(index);
        self.queue.insert(0, prompt);
        true
    }

    /// Removes the queued prompt `id`. Returns whether it was queued.
    pub(crate) fn remove(&mut self, id: u64) -> bool {
        let before = self.queue.len();
        self.queue.retain(|prompt| prompt.saved.id != id);
        self.queue.len() != before
    }

    /// The context new prompts get, even while a queued prompt is edited.
    pub(crate) fn default_attachments(&self) -> &[Attachment] {
        self.defaults
            .as_ref()
            .map_or(&self.attachments, |defaults| &defaults.attachments)
    }

    /// A queued prompt is being edited.
    pub(crate) fn editing(&self) -> bool {
        self.defaults.is_some()
    }

    /// Shows a queued prompt's images and context for editing, keeping the
    /// composer's own to return to.
    pub(crate) fn begin_edit(&mut self, images: Vec<PromptImage>, attachments: Vec<Attachment>) {
        debug_assert!(self.defaults.is_none());
        self.defaults = Some(Defaults {
            images: std::mem::replace(&mut self.images, images),
            attachments: std::mem::replace(&mut self.attachments, attachments),
        });
        self.queue_editing = true;
    }

    /// Returns to the composer's own images and context.
    pub(crate) fn end_edit(&mut self) {
        if let Some(defaults) = self.defaults.take() {
            self.images = defaults.images;
            self.attachments = defaults.attachments;
        }
        self.queue_editing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_ai::history::SavedPrompt;

    fn prompt(id: u64) -> QueuedPrompt {
        QueuedPrompt::new(
            SavedPrompt {
                id,
                text: format!("prompt {id}"),
                images: Vec::new(),
                attachments: Vec::new(),
            },
            Vec::new(),
        )
    }

    #[test]
    fn restored_queue_waits_and_continues_its_ids() {
        let composer =
            Composer::restored(Some(String::new()), Vec::new(), vec![prompt(4), prompt(2)]);
        assert!(composer.queue_paused);
        assert!(composer.draft.is_none());
        assert_eq!(composer.next_queue_id(), 5);
        assert!(!composer.dispatchable());
    }

    #[test]
    fn accepting_a_new_prompt_clears_the_draft_but_a_replacement_does_not() {
        let mut composer = Composer::default();
        assert!(composer.set_draft("text".into()));
        assert!(!composer.set_draft("text".into()));
        composer.accept(vec![prompt(1)], true);
        assert_eq!(composer.draft.as_deref(), Some("text"));
        assert_eq!(composer.next_queue_id(), 1);
        composer.accept(vec![prompt(1)], false);
        assert!(composer.draft.is_none());
        assert_eq!(composer.next_queue_id(), 2);
        assert!(!composer.is_empty());
    }

    #[test]
    fn editing_holds_dispatch_and_restores_the_defaults() {
        let mut composer = Composer {
            attachments: vec![Attachment::Group("own".into())],
            queue: vec![prompt(1), prompt(2)],
            ..Composer::default()
        };
        composer.begin_edit(Vec::new(), vec![Attachment::Group("queued".into())]);
        assert!(composer.editing());
        assert_eq!(
            composer.default_attachments(),
            [Attachment::Group("own".into())]
        );
        assert!(composer.take_next().is_none());
        let restarted = composer.restarted();
        assert!(!restarted.editing() && restarted.queue_paused);
        assert_eq!(restarted.attachments, [Attachment::Group("own".into())]);
        composer.end_edit();
        assert_eq!(composer.attachments, [Attachment::Group("own".into())]);
        assert!(composer.move_to_front(2));
        assert!(!composer.move_to_front(9));
        assert_eq!(composer.take_next().unwrap().saved.id, 2);
    }

    #[test]
    fn a_removed_prompt_leaves_the_others_in_order() {
        let mut composer = Composer {
            queue: vec![prompt(1), prompt(2), prompt(3)],
            ..Composer::default()
        };
        assert!(composer.remove(2));
        assert!(!composer.remove(2));
        let ids = composer
            .queue
            .iter()
            .map(|prompt| prompt.saved.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, [1, 3]);
    }
}
