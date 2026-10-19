//! Rebuildable metadata cache. Transcript files remain authoritative.
use super::{HistoryCodecError, SavedChat, VERSION};
use serde::{Deserialize, Serialize};
mod wire;
pub use wire::ChatSummaryWire;

/// Primitive file stamp supplied by the storage adapter, never filesystem metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub bytes: u64,
    pub modified_nanos: u128,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChatSummary {
    #[serde(default)]
    cache_version: u32,
    pub id: String,
    pub agent_id: String,
    pub title: Option<String>,
    pub name: Option<String>,
    pub pinned: bool,
    pub updated: u64,
    pub model: Option<String>,
    pub has_content: bool,
    pub bytes: u64,
    modified_nanos: u128,
}
impl ChatSummary {
    pub fn from_projection(
        id: &str,
        metadata: &super::ChatMetadata,
        has_content: bool,
        fallback: Option<String>,
    ) -> Self {
        Self {
            cache_version: 1,
            id: id.into(),
            agent_id: bounded(&metadata.agent_id),
            title: metadata.title.as_deref().map(bounded).or(fallback),
            name: metadata.name.as_deref().map(bounded),
            pinned: metadata.pinned,
            updated: metadata.updated,
            model: metadata.model.as_deref().map(bounded),
            has_content,
            bytes: 0,
            modified_nanos: 0,
        }
    }
    pub fn from_chat(chat: &SavedChat) -> Self {
        let fallback = chat
            .transcript()
            .entries
            .iter()
            .find_map(|entry| match entry {
                crate::thread::Entry::User(blocks) => blocks.iter().find_map(|block| match block {
                    crate::model::content::Content::Text(text) if !text.text.trim().is_empty() => {
                        Some(text.text.trim().chars().take(60).collect())
                    }
                    _ => None,
                }),
                _ => None,
            });
        let has_content = !chat.transcript().entries.is_empty()
            || !chat.composer().queue.is_empty()
            || chat
                .composer()
                .draft
                .as_ref()
                .is_some_and(|text| !text.is_empty());
        Self::from_projection(chat.id(), chat.metadata(), has_content, fallback)
    }
    pub fn with_stamp(mut self, stamp: FileStamp, queued: bool) -> Self {
        self.bytes = stamp.bytes;
        self.modified_nanos = stamp.modified_nanos;
        self.has_content |= queued;
        self
    }
    pub fn matches_stamp(&self, id: &str, stamp: FileStamp) -> bool {
        self.cache_version == 1
            && self.id == id
            && self.bytes == stamp.bytes
            && self.modified_nanos == stamp.modified_nanos
    }
    pub fn title(&self) -> &str {
        self.name
            .as_deref()
            .or(self.title.as_deref())
            .unwrap_or("Saved chat")
    }
}
fn bounded(text: &str) -> String {
    text.chars().take(512).collect()
}
#[derive(Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    document: Document,
}
#[derive(Deserialize)]
struct Document {
    id: String,
    metadata: super::ChatMetadata,
    transcript: Transcript,
    composer: Composer,
}
#[derive(Deserialize)]
struct Transcript {
    #[serde(deserialize_with = "entry_summary")]
    entries: EntrySummary,
}
#[derive(Deserialize)]
struct Composer {
    draft: Option<String>,
    #[serde(deserialize_with = "count")]
    queue: usize,
}
#[derive(Default)]
struct EntrySummary {
    len: usize,
    title: Option<String>,
}
fn entry_summary<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<EntrySummary, D::Error> {
    struct Summary;
    impl<'de> serde::de::Visitor<'de> for Summary {
        type Value = EntrySummary;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an array")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<EntrySummary, A::Error> {
            let mut result = EntrySummary::default();
            while let Some(entry) = seq.next_element::<TitleEntry>()? {
                result.len += 1;
                if entry.kind == "user" {
                    result.title = entry.value.0;
                }
                if result.title.is_some() {
                    while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                        result.len += 1;
                    }
                    break;
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_seq(Summary)
}
#[derive(Deserialize)]
struct TitleEntry {
    kind: String,
    value: TitleValue,
}
struct TitleValue(Option<String>);
impl<'de> Deserialize<'de> for TitleValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Text;
        impl<'de> serde::de::Visitor<'de> for Text {
            type Value = TitleValue;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("entry value")
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<TitleValue, E> {
                Ok(TitleValue(None))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<TitleValue, A::Error> {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(TitleValue(None))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<TitleValue, A::Error> {
                #[derive(Deserialize)]
                struct TextPart<'a> {
                    #[serde(default, borrow)]
                    text: Option<std::borrow::Cow<'a, str>>,
                }
                #[derive(Deserialize)]
                struct Block<'a> {
                    #[serde(borrow)]
                    kind: std::borrow::Cow<'a, str>,
                    #[serde(borrow)]
                    value: TextPart<'a>,
                }
                let mut title = None;
                while let Some(block) = seq.next_element::<Block<'de>>()? {
                    if title.is_none() && block.kind == "text" {
                        title = block
                            .value
                            .text
                            .filter(|text| !text.trim().is_empty())
                            .map(|text| text.trim().chars().take(60).collect());
                    }
                }
                Ok(TitleValue(title))
            }
        }
        deserializer.deserialize_any(Text)
    }
}
fn count<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    struct Counter;
    impl<'de> serde::de::Visitor<'de> for Counter {
        type Value = usize;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an array")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<usize, A::Error> {
            let mut len = 0;
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                len += 1;
            }
            Ok(len)
        }
    }
    deserializer.deserialize_seq(Counter)
}
pub fn decode_summary(
    bytes: &[u8],
    id: &str,
    stamp: FileStamp,
) -> Result<Option<ChatSummary>, HistoryCodecError> {
    super::check_size(bytes.len())?;
    super::preflight::check(bytes)?;
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    if envelope.format != super::FORMAT {
        return Err(HistoryCodecError::FormatMismatch);
    }
    if envelope.version != VERSION {
        return Err(HistoryCodecError::UnsupportedVersion(envelope.version));
    }
    let document = envelope.document;
    if document.id != id {
        return Err(HistoryCodecError::IdentityMismatch);
    }
    let content = document.transcript.entries.len != 0
        || document.composer.queue != 0
        || document
            .composer
            .draft
            .is_some_and(|draft| !draft.is_empty());
    if !content && document.metadata.name.is_none() && !document.metadata.pinned {
        return Ok(None);
    }
    Ok(Some(
        ChatSummary::from_projection(
            id,
            &document.metadata,
            content,
            document.transcript.entries.title,
        )
        .with_stamp(stamp, document.composer.queue != 0),
    ))
}
#[cfg(test)]
mod tests;
