//! One checked borrowed view is shared by candidate admission and exact serialization.
use super::{
    ChatMetadata, HistoryCodecError, MAX_SAVED_ENTRIES, ResumeDescriptor, SavedAttachment,
    SavedChat, SavedPrompt,
};
use crate::thread::Entry;
use std::sync::Arc;
pub trait HistoryPrompts {
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<&SavedPrompt>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
impl HistoryPrompts for Vec<SavedPrompt> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn get(&self, index: usize) -> Option<&SavedPrompt> {
        self.as_slice().get(index)
    }
}
impl HistoryPrompts for Vec<Arc<SavedPrompt>> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn get(&self, index: usize) -> Option<&SavedPrompt> {
        self.as_slice().get(index).map(AsRef::as_ref)
    }
}
impl HistoryPrompts for &[Arc<SavedPrompt>] {
    fn len(&self) -> usize {
        <[Arc<SavedPrompt>]>::len(self)
    }
    fn get(&self, index: usize) -> Option<&SavedPrompt> {
        <[Arc<SavedPrompt>]>::get(self, index).map(AsRef::as_ref)
    }
}
#[derive(Clone, Copy)]
pub struct HistoryFraming<'a> {
    pub id: &'a str,
    pub metadata: &'a ChatMetadata,
    pub resume: &'a ResumeDescriptor,
    pub draft: Option<&'a str>,
    pub attachments: &'a [SavedAttachment],
    pub times: &'a [u64],
}
#[derive(Clone, Copy, Debug)]
pub struct HistoryShape {
    pub entry_count: usize,
    pub prompt_count: usize,
}
#[derive(Clone, Copy)]
pub struct HistoryView<'a> {
    framing: HistoryFraming<'a>,
    entries: &'a [Entry],
    prompts: &'a dyn HistoryPrompts,
}
impl<'a> HistoryView<'a> {
    pub fn new(
        chat: &'a SavedChat,
        prompts: &'a dyn HistoryPrompts,
    ) -> Result<Self, HistoryCodecError> {
        let transcript = chat.transcript();
        let skip = transcript.entries.len().saturating_sub(MAX_SAVED_ENTRIES);
        let frame = HistoryFraming {
            id: chat.id(),
            metadata: chat.metadata(),
            resume: chat.resume(),
            draft: chat.composer().draft.as_deref(),
            attachments: &chat.composer().attachments,
            times: transcript.times.get(skip..).unwrap_or_default(),
        };
        Self::from_parts(frame, &transcript.entries[skip..], prompts)
    }
    pub fn from_parts(
        framing: HistoryFraming<'a>,
        entries: &'a [Entry],
        prompts: &'a dyn HistoryPrompts,
    ) -> Result<Self, HistoryCodecError> {
        if uuid::Uuid::parse_str(framing.id).is_err() {
            return Err(HistoryCodecError::InvalidId(framing.id.into()));
        }
        if entries.len() > MAX_SAVED_ENTRIES
            || framing.times.len() > MAX_SAVED_ENTRIES
            || framing.attachments.len() > 1024
            || prompts.len() > 1024
        {
            return Err(HistoryCodecError::CollectionLimit);
        }
        for index in 0..prompts.len() {
            let prompt = prompts
                .get(index)
                .ok_or(HistoryCodecError::CandidateMismatch)?;
            if prompt.images.len() > 128 || prompt.attachments.len() > 1024 {
                return Err(HistoryCodecError::CollectionLimit);
            }
        }
        Ok(Self {
            framing,
            entries,
            prompts,
        })
    }
    pub fn framing(&self) -> HistoryFraming<'a> {
        self.framing
    }
    pub fn entries(&self) -> &'a [Entry] {
        self.entries
    }
    pub fn prompts(&self) -> &'a dyn HistoryPrompts {
        self.prompts
    }
    pub fn shape(&self) -> HistoryShape {
        HistoryShape {
            entry_count: self.entries.len(),
            prompt_count: self.prompts.len(),
        }
    }
}
