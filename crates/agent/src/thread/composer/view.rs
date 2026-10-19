//! Borrowed presentation reads the actual persisted and volatile composer roots.
use super::Composer;
use crate::thread::Attachment;
use nocterm_ai::history::{
    live::inputs::LocalItemKey,
    physical::{
        AttachmentSlot, LiveAttachmentSlot, ManagedImage, PromptSlot, SnapshotOwner, TextGuard,
    },
};
use nocterm_workspace::ConnectionSummary;

pub(crate) struct ComposerView<'a> {
    decisions: &'a Composer,
    snapshot: Option<&'a SnapshotOwner>,
}
impl<'a> ComposerView<'a> {
    pub(in crate::thread) fn new(
        decisions: &'a Composer,
        snapshot: Option<&'a SnapshotOwner>,
    ) -> Self {
        Self {
            decisions,
            snapshot,
        }
    }
    pub(crate) fn draft(&self) -> Option<TextGuard<'a>> {
        self.snapshot
            .and_then(|snapshot| snapshot.composer().draft())
    }
    pub(crate) fn queue(&self) -> &'a [PromptSlot] {
        self.snapshot
            .map_or(&[], |snapshot| snapshot.composer().queue())
    }
    /// The queued prompts' visible parts, in dispatch order.
    pub(crate) fn queued_rows(&self) -> Vec<QueuedRow> {
        self.queue()
            .iter()
            .map(|prompt| QueuedRow {
                id: prompt.id(),
                text: prompt.text().to_string(),
                attachments: prompt
                    .attachments()
                    .iter()
                    .map(|attachment| match attachment {
                        AttachmentSlot::Connection(_) => {
                            Attachment::Connection(attachment.id().to_string())
                        }
                        AttachmentSlot::Group(_) => Attachment::Group(attachment.id().to_string()),
                        AttachmentSlot::LocalTerminal(_) => {
                            Attachment::UnavailableLocal(attachment.id().to_string())
                        }
                    })
                    .collect(),
            })
            .collect()
    }
    pub(crate) fn images(&self) -> &'a [ManagedImage] {
        self.snapshot
            .and_then(SnapshotOwner::live_inputs)
            .map_or(&[], |inputs| inputs.images())
    }
    pub(crate) fn attachments(&self) -> AttachmentSelection<'a> {
        match self.snapshot.and_then(SnapshotOwner::live_inputs) {
            Some(inputs) => AttachmentSelection::Live(inputs.attachments()),
            None => AttachmentSelection::Saved(
                self.snapshot
                    .map_or(&[], |snapshot| snapshot.composer().attachments()),
            ),
        }
    }
    pub(crate) fn paused(&self) -> bool {
        self.decisions.paused()
    }
    #[cfg(test)]
    pub(crate) fn next_queue_id(&self) -> u64 {
        self.decisions.next_queue_id()
    }
    #[cfg(test)]
    pub(crate) fn editing_queue(&self) -> bool {
        self.editing()
    }
    pub(crate) fn editing(&self) -> bool {
        self.snapshot
            .and_then(SnapshotOwner::live_inputs)
            .is_some_and(|inputs| inputs.editing())
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.queue().is_empty() && self.draft().is_none()
    }
    pub(crate) fn dispatchable(&self) -> bool {
        !self.queue().is_empty() && !self.paused() && !self.editing()
    }
}

/// One queued prompt as the queue list shows it, copied for a single frame.
pub(crate) struct QueuedRow {
    id: u64,
    text: String,
    attachments: Vec<Attachment>,
}
impl QueuedRow {
    pub(crate) fn id(&self) -> u64 {
        self.id
    }
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
    pub(crate) fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }
}

#[derive(Clone, Copy)]
pub(crate) enum AttachmentSelection<'a> {
    Live(&'a [LiveAttachmentSlot]),
    Saved(&'a [AttachmentSlot]),
}
impl<'a> AttachmentSelection<'a> {
    pub(crate) fn iter(self) -> AttachmentIter<'a> {
        match self {
            Self::Live(values) => AttachmentIter::Live(values.iter()),
            Self::Saved(values) => AttachmentIter::Saved(values.iter()),
        }
    }
    pub(crate) fn len(self) -> usize {
        self.iter().len()
    }
    pub(crate) fn is_empty(self) -> bool {
        self.len() == 0
    }
    pub(crate) fn contains(self, value: &Attachment) -> bool {
        self.iter().any(|slot| slot.matches(value))
    }
}
#[derive(Clone)]
pub(crate) enum AttachmentIter<'a> {
    Live(std::slice::Iter<'a, LiveAttachmentSlot>),
    Saved(std::slice::Iter<'a, AttachmentSlot>),
}
impl<'a> Iterator for AttachmentIter<'a> {
    type Item = AttachmentRef<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        Some(match self {
            Self::Live(values) => match values.next()? {
                LiveAttachmentSlot::Connection(text) => AttachmentRef::Connection(text.read()),
                LiveAttachmentSlot::Group(text) => AttachmentRef::Group(text.read()),
                LiveAttachmentSlot::UnavailableLocal(text) => {
                    AttachmentRef::UnavailableLocal(text.read())
                }
                LiveAttachmentSlot::LocalItem(key) => AttachmentRef::Terminal(*key),
            },
            Self::Saved(values) => match values.next()? {
                AttachmentSlot::Connection(text) => AttachmentRef::Connection(text.read()),
                AttachmentSlot::Group(text) => AttachmentRef::Group(text.read()),
                AttachmentSlot::LocalTerminal(text) => AttachmentRef::UnavailableLocal(text.read()),
            },
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = self.len();
        (length, Some(length))
    }
}
impl ExactSizeIterator for AttachmentIter<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Live(values) => values.len(),
            Self::Saved(values) => values.len(),
        }
    }
}
pub(crate) enum AttachmentRef<'a> {
    Terminal(LocalItemKey),
    Connection(TextGuard<'a>),
    Group(TextGuard<'a>),
    UnavailableLocal(TextGuard<'a>),
}
impl AttachmentRef<'_> {
    /// An owned attachment equal to this reference; a terminal is its item only.
    pub(crate) fn to_attachment(&self) -> Attachment {
        match self {
            Self::Terminal(key) => Attachment::Terminal(gpui_kit::EntityId::from(key.bits())),
            Self::Connection(value) => Attachment::Connection(value.to_string()),
            Self::Group(value) => Attachment::Group(value.to_string()),
            Self::UnavailableLocal(value) => Attachment::UnavailableLocal(value.to_string()),
        }
    }
    pub(crate) fn matches(&self, value: &Attachment) -> bool {
        match (self, value) {
            (Self::Terminal(left), Attachment::Terminal(right)) => left.bits() == right.as_u64(),
            (Self::Connection(left), Attachment::Connection(right))
            | (Self::Group(left), Attachment::Group(right))
            | (Self::UnavailableLocal(left), Attachment::UnavailableLocal(right)) => {
                &**left == right.as_str()
            }
            _ => false,
        }
    }
    pub(crate) fn covers_server(&self, summary: &ConnectionSummary) -> bool {
        match self {
            Self::Connection(id) => summary.id.as_ref() == &**id,
            Self::Group(group) => summary
                .group
                .as_ref()
                .is_some_and(|name| name.as_ref() == &**group),
            Self::Terminal(_) | Self::UnavailableLocal(_) => false,
        }
    }
}
