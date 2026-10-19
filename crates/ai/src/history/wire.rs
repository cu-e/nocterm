//! Authoritative borrowed version-two envelope, including bounded transcript projection.
use super::{
    FORMAT, HistoryFraming, HistoryPrompts, HistoryShape, HistoryView, MAX_SAVED_IMAGE_BYTES,
    SavedAttachment, VERSION,
};
use crate::{
    model::{
        content::{Content, MediaKind, Text},
        tools::ToolRecord,
    },
    thread::Entry,
};
use serde::{Serialize, Serializer, ser::SerializeSeq};
#[derive(Serialize)]
pub(super) struct Wire<'a> {
    format: &'static str,
    version: u32,
    document: Document<'a>,
}
#[derive(Serialize)]
struct Document<'a> {
    id: &'a str,
    metadata: &'a super::ChatMetadata,
    resume: &'a super::ResumeDescriptor,
    transcript: Transcript<'a>,
    composer: Composer<'a>,
}
#[derive(Serialize)]
struct Transcript<'a> {
    entries: Entries<'a>,
    times: &'a [u64],
}
#[derive(Serialize)]
struct Composer<'a> {
    draft: Option<&'a str>,
    queue: Prompts<'a>,
    attachments: &'a [SavedAttachment],
}
struct EmptyPrompts;
impl HistoryPrompts for EmptyPrompts {
    fn len(&self) -> usize {
        0
    }
    fn get(&self, _index: usize) -> Option<&super::SavedPrompt> {
        None
    }
}
static EMPTY_PROMPTS: EmptyPrompts = EmptyPrompts;
impl<'a> Wire<'a> {
    pub(super) fn new(view: HistoryView<'a>) -> Self {
        Self::parts(view.framing(), view.entries(), view.prompts())
    }
    fn parts(
        frame: HistoryFraming<'a>,
        entries: &'a [Entry],
        prompts: &'a dyn HistoryPrompts,
    ) -> Self {
        Self {
            format: FORMAT,
            version: VERSION,
            document: Document {
                id: frame.id,
                metadata: frame.metadata,
                resume: frame.resume,
                transcript: Transcript {
                    entries: Entries(entries),
                    times: frame.times,
                },
                composer: Composer {
                    draft: frame.draft,
                    queue: Prompts(prompts),
                    attachments: frame.attachments,
                },
            },
        }
    }
    /// Arrays/framing only; real times and metadata occur once, no fabricated prompt.
    pub(super) fn framing(frame: HistoryFraming<'a>, _shape: HistoryShape) -> Self {
        Self::parts(frame, &[], &EMPTY_PROMPTS)
    }
}
pub(super) struct Entries<'a>(pub &'a [Entry]);
impl Serialize for Entries<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for entry in self.0 {
            seq.serialize_element(&bounded(entry))?;
        }
        seq.end()
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum BorrowedEntry<'a> {
    User(Blocks<'a>),
    Agent(&'a str),
    Thought(&'a str),
    Content(Block<'a>),
    Tool(&'a ToolRecord),
}
fn bounded(entry: &Entry) -> BorrowedEntry<'_> {
    match entry {
        Entry::User(blocks) => BorrowedEntry::User(Blocks(blocks)),
        Entry::Agent(text) => BorrowedEntry::Agent(text),
        Entry::Thought(text) => BorrowedEntry::Thought(text),
        Entry::Content(content) => BorrowedEntry::Content(Block(content)),
        Entry::Tool(call) => BorrowedEntry::Tool(call),
    }
}
struct Blocks<'a>(&'a [Content]);
impl Serialize for Blocks<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for block in self.0 {
            seq.serialize_element(&Block(block))?;
        }
        seq.end()
    }
}
#[derive(Serialize)]
struct OmittedText {
    text: &'static str,
    annotations: Option<()>,
    extensions: Option<()>,
}
#[derive(Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum Omitted {
    Text(OmittedText),
}
struct Block<'a>(&'a Content);
impl Serialize for Block<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Content::Media(media)
                if media.kind == MediaKind::Image
                    && media.encoded.len() > MAX_SAVED_IMAGE_BYTES =>
            {
                Omitted::Text(OmittedText {
                    text: "[Image not kept in history]",
                    annotations: None,
                    extensions: None,
                })
                .serialize(s)
            }
            content => content.serialize(s),
        }
    }
}
struct Prompts<'a>(&'a dyn HistoryPrompts);
impl Serialize for Prompts<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for index in 0..self.0.len() {
            seq.serialize_element(self.0.get(index).ok_or_else(|| {
                <S::Error as serde::ser::Error>::custom("candidate prompt sequence changed")
            })?)?;
        }
        seq.end()
    }
}
pub(super) fn entry_measure(entry: &Entry) -> Result<super::FormatSize, super::MeasurementError> {
    super::size::measure(&bounded(entry))
}
pub(super) fn clone_entries(entries: &[Entry]) -> Vec<Entry> {
    entries[entries.len().saturating_sub(super::MAX_SAVED_ENTRIES)..]
        .iter()
        .map(|entry| match entry {
            Entry::User(blocks) => Entry::User(blocks.iter().map(clone_block).collect()),
            Entry::Content(content) => Entry::Content(clone_block(content)),
            entry => entry.clone(),
        })
        .collect()
}
fn clone_block(content: &Content) -> Content {
    match content {
        Content::Media(media)
            if media.kind == MediaKind::Image && media.encoded.len() > MAX_SAVED_IMAGE_BYTES =>
        {
            Content::Text(Text {
                text: "[Image not kept in history]".into(),
                annotations: None,
                extensions: None,
            })
        }
        content => content.clone(),
    }
}
