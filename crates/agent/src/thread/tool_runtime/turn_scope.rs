//! One active-turn grant aliases its original dispatch and keeps only revocation decisions.
use super::super::{
    Attachment,
    composer::{AttachmentIter, AttachmentRef, AttachmentSelection},
};
use nocterm_ai::history::dispatch::ActivePromptScope;

pub(super) struct TurnScope {
    original: ActivePromptScope,
    excluded: [u64; 16],
}
impl TurnScope {
    pub(super) fn new(original: ActivePromptScope) -> Self {
        Self {
            original,
            excluded: [0; 16],
        }
    }
    pub(super) fn attachments(&self) -> TurnAttachments<'_> {
        TurnAttachments {
            values: selection(&self.original).iter(),
            excluded: Some(&self.excluded),
            index: 0,
        }
    }
    pub(super) fn detach(&mut self, value: &Attachment) {
        for (index, slot) in selection(&self.original).iter().enumerate() {
            if slot.matches(value)
                && let Some(word) = self.excluded.get_mut(index / 64)
            {
                *word |= 1 << (index % 64);
            }
        }
    }
}
fn selection(scope: &ActivePromptScope) -> AttachmentSelection<'_> {
    match scope.live_attachments() {
        Some(values) => AttachmentSelection::Live(values),
        None => AttachmentSelection::Saved(scope.saved_attachments()),
    }
}
pub(in crate::thread) struct TurnAttachments<'a> {
    values: AttachmentIter<'a>,
    excluded: Option<&'a [u64; 16]>,
    index: usize,
}
impl<'a> TurnAttachments<'a> {
    pub(super) fn defaults(values: AttachmentSelection<'a>) -> Self {
        Self {
            values: values.iter(),
            excluded: None,
            index: 0,
        }
    }
    pub(in crate::thread) fn contains(mut self, value: &Attachment) -> bool {
        self.any(|slot| slot.matches(value))
    }
}
impl<'a> Iterator for TurnAttachments<'a> {
    type Item = AttachmentRef<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let value = self.values.next()?;
            let index = self.index;
            self.index += 1;
            let excluded = self
                .excluded
                .and_then(|bits| bits.get(index / 64))
                .is_some_and(|word| word & (1 << (index % 64)) != 0);
            if !excluded {
                return Some(value);
            }
        }
    }
}
