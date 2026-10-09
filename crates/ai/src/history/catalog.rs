//! Rebuildable metadata cache. Transcript files remain authoritative.
use super::{MAX_FILE_BYTES, MAX_RESTORED, SavedChat, VERSION, file, valid_id};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MAX_METADATA_BYTES: u64 = 16 * 1024;

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
    pub fn from_chat(chat: &SavedChat) -> Self {
        let fallback = chat.entries.iter().find_map(|entry| match entry {
            crate::thread::Entry::User(blocks) => blocks.iter().find_map(|block| match block {
                crate::acp::ContentBlock::Text(text) if !text.text.trim().is_empty() => {
                    Some(text.text.trim().chars().take(60).collect())
                }
                _ => None,
            }),
            _ => None,
        });
        Self {
            cache_version: 1,
            id: chat.id.clone(),
            agent_id: bounded(&chat.agent_id),
            title: chat.title.as_deref().map(bounded).or(fallback),
            name: chat.name.as_deref().map(bounded),
            pinned: chat.pinned,
            updated: chat.updated,
            model: chat.model.as_deref().map(bounded),
            has_content: !chat.entries.is_empty()
                || !chat.queue.is_empty()
                || chat.draft.as_ref().is_some_and(|text| !text.is_empty()),
            bytes: 0,
            modified_nanos: 0,
        }
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
fn cache_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.meta"))
}
fn stamp(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_nanos())
}
pub(super) fn update_cache(dir: &Path, chat: &SavedChat, queued: bool) {
    let Some(path) = file(dir, &chat.id) else {
        return;
    };
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let mut summary = ChatSummary::from_chat(chat);
    summary.has_content |= queued;
    summary.bytes = metadata.len();
    summary.modified_nanos = stamp(&metadata);
    write_cache(dir, &summary);
}
fn write_cache(dir: &Path, summary: &ChatSummary) {
    if let Ok(text) = serde_json::to_string(summary) {
        let _ = nocterm_core::persist::save_text(&cache_path(dir, &summary.id), &text);
    }
}
pub(super) fn delete_cache(dir: &Path, id: &str) {
    let _ = fs::remove_file(cache_path(dir, id));
}

/// Only the best 200 small rows survive the scan; no transcript is retained.
pub fn load_catalog(dir: &Path) -> Vec<ChatSummary> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().is_none_or(|ext| ext != "json") || !valid_id(id) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let cached = fs::File::open(cache_path(dir, id))
            .ok()
            .and_then(|file| {
                serde_json::from_reader::<_, ChatSummary>(BufReader::new(
                    file.take(MAX_METADATA_BYTES),
                ))
                .ok()
            })
            .filter(|row| {
                row.cache_version == 1
                    && row.id == id
                    && row.bytes == metadata.len()
                    && row.modified_nanos == stamp(&metadata)
            });
        let summary =
            cached.or_else(|| legacy(&path, id, &metadata).inspect(|row| write_cache(dir, row)));
        if let Some(summary) =
            summary.filter(|row| row.has_content || row.name.is_some() || row.pinned)
        {
            rows.push(summary);
            rows.sort_by_key(|row| std::cmp::Reverse((row.pinned, row.updated)));
            rows.truncate(MAX_RESTORED);
        }
    }
    rows
}

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
fn legacy(path: &Path, id: &str, metadata: &fs::Metadata) -> Option<ChatSummary> {
    let bytes = super::preflight::read(path).ok()?;
    super::preflight::check(&bytes).ok()?;
    let old: Legacy = serde_json::from_slice(&bytes).ok()?;
    if old.version != VERSION || old.id != id {
        return None;
    }
    let content =
        old.entries.len != 0 || old.queue != 0 || old.draft.is_some_and(|draft| !draft.is_empty());
    if !content && old.name.is_none() && !old.pinned {
        return None;
    }
    Some(ChatSummary {
        cache_version: 1,
        id: old.id,
        agent_id: bounded(&old.agent_id),
        title: old.title.as_deref().map(bounded).or(old.entries.title),
        name: old.name.as_deref().map(bounded),
        pinned: old.pinned,
        updated: old.updated,
        model: old.model.as_deref().map(bounded),
        has_content: content,
        bytes: metadata.len(),
        modified_nanos: stamp(metadata),
    })
}

/// Opening one row reads a bounded file, never the whole archive.
pub fn load(dir: &Path, id: &str) -> Result<SavedChat, String> {
    let path = file(dir, id).ok_or("invalid chat id")?;
    let bytes = super::preflight::read(&path)?;
    let resident = super::preflight::check(&bytes)?;
    let mut chat: SavedChat = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if chat.version != VERSION || chat.id != id {
        return Err("Saved chat identity or version does not match.".into());
    }
    let mut prepared = super::PreparedBudget::new(&chat)?;
    prepared.resident_bytes = resident;
    chat.prepared = Some(prepared);
    Ok(chat)
}

#[cfg(test)]
mod tests;
