//! Borrowed sidecar projection applies the same summary policy to actual owners.
use crate::history::{
    FileStamp, SavedDocument,
    physical::{ContentSlot, RowBody, SnapshotOwner},
};
use serde::{Serialize, Serializer};

enum Source<'a> {
    Managed(&'a SnapshotOwner),
    Native(&'a SavedDocument),
}
pub struct ChatSummaryWire<'a> {
    source: Source<'a>,
    stamp: FileStamp,
}
impl<'a> ChatSummaryWire<'a> {
    pub fn from_managed(snapshot: &'a SnapshotOwner, stamp: FileStamp) -> Self {
        Self {
            source: Source::Managed(snapshot),
            stamp,
        }
    }
    pub fn from_document(document: &'a SavedDocument, stamp: FileStamp) -> Self {
        Self {
            source: Source::Native(document),
            stamp,
        }
    }
}
#[derive(Serialize)]
struct Fields<'a> {
    cache_version: u32,
    id: &'a str,
    agent_id: &'a str,
    title: Option<&'a str>,
    name: Option<&'a str>,
    pinned: bool,
    updated: u64,
    model: Option<&'a str>,
    has_content: bool,
    bytes: u64,
    modified_nanos: u128,
}
fn prefix(text: &str, chars: usize) -> &str {
    &text[..text
        .char_indices()
        .nth(chars)
        .map_or(text.len(), |(offset, _)| offset)]
}
impl Serialize for ChatSummaryWire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.source {
            Source::Managed(snapshot) => self.serialize_managed(snapshot, serializer),
            Source::Native(document) => self.serialize_native(document, serializer),
        }
    }
}
impl ChatSummaryWire<'_> {
    fn serialize_managed<S: Serializer>(
        &self,
        snapshot: &SnapshotOwner,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let metadata = snapshot.metadata();
        let id = snapshot.id();
        let agent = metadata.agent_id();
        let title = metadata.title();
        let name = metadata.name();
        let model = metadata.model();
        let fallback = snapshot.rows().iter().find_map(|row| match row.view() {
            RowBody::User(blocks) => blocks.contents().find_map(|content| match content {
                ContentSlot::Text(text) => {
                    let text = text.text();
                    (!text.trim().is_empty()).then_some(text)
                }
                _ => None,
            }),
            _ => None,
        });
        let composer = snapshot.composer();
        Fields {
            cache_version: 1,
            id: &id,
            agent_id: prefix(&agent, 512),
            title: title
                .as_deref()
                .map(|value| prefix(value, 512))
                .or_else(|| fallback.as_deref().map(|value| prefix(value.trim(), 60))),
            name: name.as_deref().map(|value| prefix(value, 512)),
            pinned: metadata.pinned(),
            updated: metadata.updated(),
            model: model.as_deref().map(|value| prefix(value, 512)),
            has_content: !snapshot.rows().is_empty()
                || !composer.queue().is_empty()
                || composer.draft().is_some_and(|text| !text.is_empty()),
            bytes: self.stamp.bytes,
            modified_nanos: self.stamp.modified_nanos,
        }
        .serialize(serializer)
    }
    fn serialize_native<S: Serializer>(
        &self,
        document: &SavedDocument,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let metadata = &document.metadata;
        let fallback = document
            .transcript
            .entries
            .iter()
            .find_map(|row| match row {
                crate::thread::Entry::User(blocks) => {
                    blocks.iter().find_map(|content| match content {
                        crate::model::content::Content::Text(text)
                            if !text.text.trim().is_empty() =>
                        {
                            Some(prefix(text.text.trim(), 60))
                        }
                        _ => None,
                    })
                }
                _ => None,
            });
        Fields {
            cache_version: 1,
            id: &document.id,
            agent_id: prefix(&metadata.agent_id, 512),
            title: metadata
                .title
                .as_deref()
                .map(|value| prefix(value, 512))
                .or(fallback),
            name: metadata.name.as_deref().map(|value| prefix(value, 512)),
            pinned: metadata.pinned,
            updated: metadata.updated,
            model: metadata.model.as_deref().map(|value| prefix(value, 512)),
            has_content: !document.transcript.entries.is_empty()
                || !document.composer.queue.is_empty()
                || document
                    .composer
                    .draft
                    .as_ref()
                    .is_some_and(|text| !text.is_empty()),
            bytes: self.stamp.bytes,
            modified_nanos: self.stamp.modified_nanos,
        }
        .serialize(serializer)
    }
}
