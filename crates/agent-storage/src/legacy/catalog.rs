//! Private streaming version-one sidecar rebuild; never rewrites transcript bytes.
use nocterm_ai::history::{ChatSummary, FileStamp, HistoryCodecError};
use serde::Deserialize;
#[derive(Deserialize)]
struct Legacy {
    version: u32,
    id: String,
    agent_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pinned: bool,
    updated: u64,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    draft: Option<String>,
    #[serde(deserialize_with = "entry_summary", default)]
    entries: EntrySummary,
    #[serde(deserialize_with = "count", default)]
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
                struct Block<'a> {
                    #[serde(rename = "type", borrow)]
                    kind: std::borrow::Cow<'a, str>,
                    #[serde(default, borrow)]
                    text: Option<std::borrow::Cow<'a, str>>,
                }
                let mut title = None;
                while let Some(block) = seq.next_element::<Block<'de>>()? {
                    if title.is_none() && block.kind == "text" {
                        title = block
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
pub(crate) fn decode_summary(
    bytes: &[u8],
    id: &str,
    stamp: FileStamp,
) -> Result<Option<ChatSummary>, HistoryCodecError> {
    if bytes.len() as u64 > nocterm_ai::history::MAX_FILE_BYTES {
        return Err(HistoryCodecError::EncodedLimit);
    }
    nocterm_ai::history::structural_bytes(bytes)?;
    let old: Legacy = serde_json::from_slice(bytes)?;
    if old.version != 1 {
        return Err(HistoryCodecError::UnsupportedVersion(old.version));
    }
    if old.id != id {
        return Err(HistoryCodecError::IdentityMismatch);
    }
    let content =
        old.entries.len != 0 || old.queue != 0 || old.draft.is_some_and(|draft| !draft.is_empty());
    if !content && old.name.is_none() && !old.pinned {
        return Ok(None);
    }
    let metadata = nocterm_ai::history::ChatMetadata {
        agent_id: old.agent_id,
        title: old.title,
        name: old.name,
        pinned: old.pinned,
        updated: old.updated,
        model: old.model,
    };
    Ok(Some(
        ChatSummary::from_projection(&old.id, &metadata, content, old.entries.title)
            .with_stamp(stamp, old.queue != 0),
    ))
}
