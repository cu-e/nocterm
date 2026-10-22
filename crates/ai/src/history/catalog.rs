//! Rebuildable metadata cache. Transcript files remain authoritative.
use super::HistoryCodecError;
use nocterm_project_model::ContextId;
use serde::Deserialize;

/// Primitive file stamp supplied by the storage adapter, never filesystem metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub bytes: u64,
    pub modified_nanos: u128,
}

pub use super::catalog_header::ChatSummary;
#[derive(Deserialize)]
struct Legacy {
    version: u32,
    #[serde(default)]
    scope: Option<ContextId>,
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
pub fn decode_summary(
    bytes: &[u8],
    id: &str,
    stamp: FileStamp,
) -> Result<Option<ChatSummary>, HistoryCodecError> {
    super::check_size(bytes.len())?;
    super::preflight::check(bytes)?;
    if bytes.starts_with(super::prefix::START) {
        let header = super::decode_catalog_header(bytes, id)?;
        let envelope: ScopedSummary = serde_json::from_slice(bytes)?;
        if envelope.version != super::VERSION
            || envelope.chat.version != super::VERSION
            || envelope.catalog != header
            || envelope.chat.catalog(id)? != header
        {
            return Err(HistoryCodecError::HeaderMismatch);
        }
        return Ok(header.summary(stamp));
    }
    let old: Legacy = serde_json::from_slice(bytes)?;
    if old.version != super::LEGACY_VERSION {
        super::header::resolve_scope(old.version, old.scope)?;
        return Err(HistoryCodecError::MissingCatalog);
    }
    Ok(old
        .catalog(id)?
        .summary(stamp)
        .map(|row| row.with_version(super::TranscriptVersion::LEGACY)))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopedSummary {
    version: u32,
    catalog: super::CatalogHeader,
    chat: Legacy,
}
impl Legacy {
    fn catalog(self, id: &str) -> Result<super::CatalogHeader, HistoryCodecError> {
        let scope = super::header::resolve_scope(self.version, self.scope)?;
        if self.id != id {
            return Err(HistoryCodecError::IdentityMismatch);
        }
        let content = self.entries.len != 0
            || self.queue != 0
            || self.draft.is_some_and(|draft| !draft.is_empty());
        let saved = super::SavedHeader {
            id: self.id,
            agent_id: self.agent_id,
            scope,
            title: self.title.or(self.entries.title),
            name: self.name,
            pinned: self.pinned,
            updated: self.updated,
        };
        Ok(super::CatalogHeader::from_parts(
            &saved,
            &self.model,
            &super::catalog_header::Preview::for_title(None, content),
            None,
            false,
        ))
    }
}

#[cfg(test)]
mod tests;
