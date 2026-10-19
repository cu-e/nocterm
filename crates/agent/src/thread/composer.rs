//! Queue-dispatch decisions apart from the document: whether queued prompts wait
//! for the user, and the id the next prompt takes. Drafts, queued prompts, images
//! and attachments live in the Live document and are read through `ComposerView`.
mod view;
pub(crate) use view::{
    AttachmentIter, AttachmentRef, AttachmentSelection, ComposerView, QueuedRow,
};

pub(crate) struct Composer {
    queue_paused: bool,
    next_queue_id: u64,
}

impl Default for Composer {
    fn default() -> Self {
        Self {
            queue_paused: false,
            next_queue_id: 1,
        }
    }
}

impl Composer {
    pub(crate) fn paused(&self) -> bool {
        self.queue_paused
    }
    pub(super) fn pause(&mut self) {
        self.queue_paused = true;
    }
    pub(super) fn resume(&mut self) {
        self.queue_paused = false;
    }
    /// The id the next new queued prompt takes.
    pub(crate) fn next_queue_id(&self) -> u64 {
        self.next_queue_id
    }
    /// A prompt was queued; a replacement keeps the id sequence.
    pub(super) fn queued(&mut self, replaced: bool) {
        if !replaced {
            self.next_queue_id += 1;
        }
    }
    /// A saved chat's queued prompts wait until the user resumes them, and new
    /// ids continue after the highest one already queued.
    pub(super) fn restored(highest_queued: Option<u64>) -> Self {
        Self {
            queue_paused: true,
            next_queue_id: highest_queued.unwrap_or(0) + 1,
        }
    }
    /// The state a replacement agent process starts from: paused.
    pub(crate) fn restarted(&self) -> Self {
        Self {
            queue_paused: true,
            next_queue_id: self.next_queue_id,
        }
    }
    pub(super) fn reset_after_disable(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_queue_waits_and_continues_its_ids() {
        let composer = Composer::restored(Some(4));
        assert!(composer.paused());
        assert_eq!(composer.next_queue_id(), 5);
    }

    #[test]
    fn a_replacement_keeps_the_id_sequence() {
        let mut composer = Composer::default();
        composer.queued(true);
        assert_eq!(composer.next_queue_id(), 1);
        composer.queued(false);
        assert_eq!(composer.next_queue_id(), 2);
    }

    #[test]
    fn restarting_pauses_and_keeps_ids() {
        let mut composer = Composer::default();
        composer.queued(false);
        let restarted = composer.restarted();
        assert!(restarted.paused());
        assert_eq!(restarted.next_queue_id(), 2);
    }
}
