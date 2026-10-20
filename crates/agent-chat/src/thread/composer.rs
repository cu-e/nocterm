//! What the user is composing, apart from the conversation it goes into: the
//! unsent draft, the context and images for the next prompt, and the queue of
//! prompts waiting their turn.
pub(in crate::thread) mod prompt_preparation;
use super::{Attachment, queue::QueuedPrompt};
use nocterm_ai::images::PromptImage;

pub struct Composer {
    draft: Option<std::sync::Arc<String>>,
    attachments: Vec<Attachment>,
    images: Vec<PromptImage>,
    queue: Vec<QueuedPrompt>,
    queue_paused: bool,
    /// Composer edits suspend dispatch independently of stops and errors.
    queue_editing: bool,
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

pub(super) struct PromptCandidate {
    queue: Vec<QueuedPrompt>,
    replaced: bool,
}
impl PromptCandidate {
    pub(super) fn queue(&self) -> &[QueuedPrompt] {
        &self.queue
    }
}
impl Composer {
    pub(super) fn prepare_submission(
        &self,
        text: String,
        replace: Option<u64>,
        attachments: Vec<nocterm_ai::history::SavedAttachment>,
    ) -> Result<PromptCandidate, super::queue::SubmitError> {
        let mut saved = nocterm_ai::history::SavedPrompt {
            id: replace.unwrap_or(self.next_queue_id()),
            text,
            images: Vec::new(),
            attachments,
        };
        let encoded_len = nocterm_ai::history::prompt_size(&saved)?
            + self
                .images()
                .iter()
                .map(|image| image.encoded_len())
                .sum::<usize>()
            + self.images().len().saturating_sub(1);
        saved.images = self.images().iter().map(|image| image.content()).collect();
        let prompt = QueuedPrompt::prepared(saved, self.attachments().to_vec(), Some(encoded_len));
        let candidate = self.prepare_prompt_change(prompt, replace)?;
        let attachment_count = candidate
            .queue()
            .iter()
            .map(|prompt| prompt.saved.attachments.len())
            .max()
            .unwrap_or(0);
        if candidate.queue().len() > 1024 || self.images().len() > 128 || attachment_count > 1024 {
            return Err(super::queue::SubmitError::TooMany);
        }
        Ok(candidate)
    }

    pub fn draft(&self) -> Option<&String> {
        self.draft.as_deref()
    }
    pub(super) fn shared_draft(&self) -> Option<std::sync::Arc<String>> {
        self.draft.clone()
    }
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }
    pub fn images(&self) -> &[PromptImage] {
        &self.images
    }
    pub fn queue(&self) -> &[QueuedPrompt] {
        &self.queue
    }
    pub fn paused(&self) -> bool {
        self.queue_paused
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn editing_queue(&self) -> bool {
        self.queue_editing
    }
    pub(super) fn pause(&mut self) {
        self.queue_paused = true;
    }
    pub(super) fn resume(&mut self) {
        self.queue_paused = false;
    }
    pub(super) fn toggle_attachment(&mut self, attachment: Attachment) -> bool {
        if let Some(index) = self
            .attachments
            .iter()
            .position(|value| value == &attachment)
        {
            self.attachments.remove(index);
            true
        } else {
            self.attachments.push(attachment);
            false
        }
    }
    pub(super) fn add_attachment_if_absent(&mut self, attachment: Attachment) {
        if !self.attachments.contains(&attachment) {
            self.attachments.push(attachment);
        }
    }
    pub(super) fn replace_context(&mut self, attachments: Vec<Attachment>) {
        self.attachments = attachments;
    }
    pub(super) fn clear_context(&mut self) {
        self.attachments.clear();
    }
    pub(super) fn reset_after_disable(&mut self) {
        self.clear_context();
        self.images.clear();
    }
    pub(super) fn remove_image(&mut self, index: usize) -> bool {
        if index >= self.images.len() {
            return false;
        }
        self.images.remove(index);
        true
    }
    pub(super) fn replace_images(
        &mut self,
        images: Vec<PromptImage>,
    ) -> Result<(), nocterm_ai::images::ImageCollectionError> {
        nocterm_ai::images::validate_collection(&images)?;
        self.images = images;
        Ok(())
    }
    pub(super) fn prepare_prompt_change(
        &self,
        prompt: QueuedPrompt,
        replace: Option<u64>,
    ) -> Result<PromptCandidate, super::queue::SubmitError> {
        let mut queue = self.queue.clone();
        if let Some(id) = replace {
            let Some(slot) = queue.iter_mut().find(|value| value.saved.id == id) else {
                return Err(super::queue::SubmitError::AlreadySent);
            };
            *slot = prompt;
        } else {
            queue.push(prompt);
        }
        Ok(PromptCandidate {
            queue,
            replaced: replace.is_some(),
        })
    }
    pub(super) fn commit_prompt_change(&mut self, candidate: PromptCandidate) {
        self.accept(candidate.queue, candidate.replaced);
    }
    /// A saved chat's composer. Its queue waits until the user resumes it.
    pub fn restored(
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
            draft: draft
                .filter(|draft| !draft.is_empty())
                .map(std::sync::Arc::new),
            attachments,
            queue,
            queue_paused: true,
            next_queue_id,
            ..Self::default()
        }
    }

    /// The state a replacement agent process starts from: paused, and out of
    /// any edit of a queued prompt.
    pub fn restarted(&self) -> Self {
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
    pub fn is_empty(&self) -> bool {
        self.draft.is_none() && self.queue.is_empty()
    }

    /// Sets the draft; an empty text clears it. Returns whether it changed.
    pub fn set_draft(&mut self, text: String) -> bool {
        let draft = (!text.is_empty()).then(|| std::sync::Arc::new(text));
        if self.draft == draft {
            return false;
        }
        self.draft = draft;
        true
    }

    /// The id the next new queued prompt takes.
    pub fn next_queue_id(&self) -> u64 {
        self.next_queue_id
    }

    /// The queue changed and its prompts wait for dispatch.
    fn accept(&mut self, queue: Vec<QueuedPrompt>, replaced: bool) {
        self.queue = queue;
        if !replaced {
            self.draft = None;
            self.next_queue_id += 1;
        }
        self.images.clear();
    }

    /// Queued prompts wait to be sent and nothing holds them back.
    pub fn dispatchable(&self) -> bool {
        !self.queue.is_empty() && !self.paused() && !self.queue_editing
    }

    /// Takes the next prompt to send, when dispatch is not held back.
    pub fn take_next(&mut self) -> Option<QueuedPrompt> {
        self.dispatchable().then(|| self.queue.remove(0))
    }

    /// Moves the queued prompt `id` to the front. Returns whether it is queued.
    pub fn move_to_front(&mut self, id: u64) -> bool {
        let Some(index) = self.queue.iter().position(|prompt| prompt.saved.id == id) else {
            return false;
        };
        let prompt = self.queue.remove(index);
        self.queue.insert(0, prompt);
        true
    }

    /// Removes the queued prompt `id`. Returns whether it was queued.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.queue.len();
        self.queue.retain(|prompt| prompt.saved.id != id);
        self.queue.len() != before
    }

    /// The context new prompts get, even while a queued prompt is edited.
    pub fn default_attachments(&self) -> &[Attachment] {
        self.defaults
            .as_ref()
            .map_or(&self.attachments, |defaults| &defaults.attachments)
    }

    /// A queued prompt is being edited.
    pub fn editing(&self) -> bool {
        self.defaults.is_some()
    }

    /// Shows a queued prompt's images and context for editing, keeping the
    /// composer's own to return to.
    pub fn begin_edit(&mut self, images: Vec<PromptImage>, attachments: Vec<Attachment>) {
        debug_assert!(self.defaults.is_none());
        self.defaults = Some(Defaults {
            images: std::mem::replace(&mut self.images, images),
            attachments: std::mem::replace(&mut self.attachments, attachments),
        });
        self.queue_editing = true;
    }

    /// Returns to the composer's own images and context.
    pub fn end_edit(&mut self) {
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
        assert!(composer.paused());
        assert!(composer.draft().is_none());
        assert_eq!(composer.next_queue_id(), 5);
        assert!(!composer.dispatchable());
    }

    #[test]
    fn accepting_a_new_prompt_clears_the_draft_but_a_replacement_does_not() {
        let mut composer = Composer::default();
        let candidate = composer.prepare_prompt_change(prompt(1), None).unwrap();
        composer.commit_prompt_change(candidate);
        assert!(composer.set_draft("text".into()));
        assert!(!composer.set_draft("text".into()));
        let replaced = composer.prepare_prompt_change(prompt(1), Some(1)).unwrap();
        composer.commit_prompt_change(replaced);
        assert_eq!(composer.draft().map(String::as_str), Some("text"));
        assert_eq!(composer.next_queue_id(), 2);
        let next = composer.prepare_prompt_change(prompt(2), None).unwrap();
        composer.commit_prompt_change(next);
        assert!(composer.draft().is_none());
        assert_eq!(composer.next_queue_id(), 3);
        assert!(!composer.is_empty());
    }

    #[test]
    fn editing_holds_dispatch_and_restores_the_defaults() {
        let mut composer = Composer::restored(
            None,
            vec![Attachment::Group("own".into())],
            vec![prompt(1), prompt(2)],
        );
        composer.resume();
        composer.begin_edit(Vec::new(), vec![Attachment::Group("queued".into())]);
        assert!(composer.editing());
        assert_eq!(
            composer.default_attachments(),
            [Attachment::Group("own".into())]
        );
        assert!(composer.take_next().is_none());
        let restarted = composer.restarted();
        assert!(!restarted.editing() && restarted.paused());
        assert_eq!(restarted.attachments(), [Attachment::Group("own".into())]);
        composer.end_edit();
        assert_eq!(composer.attachments(), [Attachment::Group("own".into())]);
        assert!(composer.move_to_front(2));
        assert!(!composer.move_to_front(9));
        assert_eq!(composer.take_next().unwrap().saved.id, 2);
    }

    #[test]
    fn rejected_or_discarded_candidates_preserve_the_draft_queue_and_next_id() {
        let mut composer = Composer::restored(None, Vec::new(), vec![prompt(1)]);
        composer.set_draft("keep this draft".into());
        let next_id = composer.next_queue_id();
        assert!(matches!(
            composer.prepare_prompt_change(prompt(99), Some(99)),
            Err(super::super::queue::SubmitError::AlreadySent)
        ));
        let candidate = composer
            .prepare_prompt_change(prompt(next_id), None)
            .unwrap();
        assert_eq!(candidate.queue().len(), 2);
        drop(candidate);
        assert_eq!(composer.queue().len(), 1);
        assert_eq!(composer.queue()[0].saved.id, 1);
        assert_eq!(
            composer.draft().map(String::as_str),
            Some("keep this draft")
        );
        assert_eq!(composer.next_queue_id(), next_id);
        assert!(composer.paused());
    }

    #[test]
    fn rejected_image_collection_preserves_existing_images_and_edit_defaults() {
        let bytes = nocterm_ai::testing::images::png_header_fixture(2, 2, 33);
        let codec = nocterm_ai::testing::ScriptedImageCodec::default();
        codec.inspection(
            &bytes,
            Ok(
                nocterm_ai::images::ImageFacts::new(nocterm_ai::images::ImageFormat::Png, 2, 2)
                    .unwrap(),
            ),
        );
        let image = PromptImage::validate(bytes, &codec).unwrap();
        let mut composer = Composer::default();
        composer.replace_images(vec![image.clone()]).unwrap();
        composer.replace_context(vec![Attachment::Group("own".into())]);
        composer.begin_edit(
            vec![image.clone()],
            vec![Attachment::Group("queued".into())],
        );
        assert!(
            composer
                .replace_images(vec![image; nocterm_ai::images::MAX_IMAGES + 1])
                .is_err()
        );
        assert_eq!(composer.images().len(), 1);
        assert!(composer.editing());
        composer.end_edit();
        assert_eq!(composer.images().len(), 1);
        assert_eq!(composer.attachments(), [Attachment::Group("own".into())]);
    }

    #[test]
    fn a_removed_prompt_leaves_the_others_in_order() {
        let mut composer =
            Composer::restored(None, Vec::new(), vec![prompt(1), prompt(2), prompt(3)]);
        assert!(composer.remove(2));
        assert!(!composer.remove(2));
        let ids = composer
            .queue()
            .iter()
            .map(|prompt| prompt.saved.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, [1, 3]);
    }
}
