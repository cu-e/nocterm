//! Version-one decoding preserves the original field grammar without serde flatten buffers.
use super::content_decode::Block;
use nocterm_ai::{
    history::{SavedAttachment, bounded_collection},
    session_config::Choices,
};
use serde::{
    Deserialize, Serialize,
    de::{MapAccess, Visitor},
    ser::SerializeMap,
};
use std::{fmt, path::PathBuf};
#[derive(Debug)]
pub(super) struct LegacyChat {
    metadata: Metadata,
    resume: Resume,
    transcript: Transcript,
    composer: Composer,
}
#[derive(Debug, Default)]
struct Metadata {
    version: u32,
    id: String,
    agent_id: String,
    title: Option<String>,
    name: Option<String>,
    pinned: bool,
    updated: u64,
}
#[derive(Debug, Default)]
struct Resume {
    session_id: Option<String>,
    workdir: Option<PathBuf>,
    model: Option<String>,
    config: Choices,
    mode: Option<String>,
    pending_history: bool,
}
#[derive(Debug, Default)]
struct Transcript {
    entries: Vec<LegacyEntry>,
    times: Vec<u64>,
}
#[derive(Debug, Default)]
struct Composer {
    draft: Option<String>,
    queue: Vec<LegacyPrompt>,
    attachments: Vec<SavedAttachment>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum LegacyEntry {
    User(Vec<Block>),
    Agent(String),
    Thought(String),
    Content(Block),
    Tool(super::tools_decode::Call),
}
#[derive(Debug, Serialize, Deserialize)]
struct LegacyPrompt {
    id: u64,
    text: String,
    #[serde(default, deserialize_with = "images")]
    images: Vec<Block>,
    #[serde(default, deserialize_with = "attachments")]
    attachments: Vec<SavedAttachment>,
}
fn images<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Block>, D::Error> {
    bounded_collection::<_, _, 128>(d)
}
fn attachments<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<SavedAttachment>, D::Error> {
    bounded_collection::<_, _, 1024>(d)
}
struct Entries(Vec<LegacyEntry>);
impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        bounded_collection::<_, _, 2000>(d).map(Self)
    }
}
struct Prompts(Vec<LegacyPrompt>);
impl<'de> Deserialize<'de> for Prompts {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        bounded_collection::<_, _, 1024>(d).map(Self)
    }
}
struct Attachments(Vec<SavedAttachment>);
impl<'de> Deserialize<'de> for Attachments {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        attachments(d).map(Self)
    }
}
struct Times(Vec<u64>);
impl<'de> Deserialize<'de> for Times {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        bounded_collection::<_, _, 2000>(d).map(Self)
    }
}
/// Tracks duplicates separately from optional values, including explicit null.
struct Fields(u32);
impl Fields {
    fn accept<E: serde::de::Error>(&mut self, bit: u32, name: &str) -> Result<(), E> {
        if self.0 & bit != 0 {
            return Err(E::custom(format!("duplicate legacy field {name}")));
        }
        self.0 |= bit;
        Ok(())
    }
    fn required<E: serde::de::Error>(&self, bit: u32, name: &str) -> Result<(), E> {
        if self.0 & bit == 0 {
            return Err(E::custom(format!("missing legacy field {name}")));
        }
        Ok(())
    }
}
impl<'de> Deserialize<'de> for LegacyChat {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(Document)
    }
}
struct Document;
impl<'de> Visitor<'de> for Document {
    type Value = LegacyChat;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a legacy version-one chat")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<LegacyChat, A::Error> {
        let mut chat = LegacyChat {
            metadata: Metadata::default(),
            resume: Resume::default(),
            transcript: Transcript::default(),
            composer: Composer::default(),
        };
        let mut seen = Fields(0);
        while let Some(key) = map.next_key::<String>()? {
            chat.read_field(&mut seen, &key, &mut map)?;
        }
        for (bit, name) in [
            (1, "version"),
            (2, "id"),
            (4, "agent_id"),
            (8192, "updated"),
            (16384, "entries"),
        ] {
            seen.required(bit, name)?;
        }
        Ok(chat)
    }
}
impl LegacyChat {
    /// Identity, metadata and resume fields.
    fn read_field<'de, A: MapAccess<'de>>(
        &mut self,
        seen: &mut Fields,
        key: &str,
        map: &mut A,
    ) -> Result<(), A::Error> {
        match key {
            "version" => {
                seen.accept(1, key)?;
                self.metadata.version = map.next_value()?;
            }
            "id" => {
                seen.accept(2, key)?;
                self.metadata.id = map.next_value()?;
            }
            "agent_id" => {
                seen.accept(4, key)?;
                self.metadata.agent_id = map.next_value()?;
            }
            "title" => {
                seen.accept(8, key)?;
                self.metadata.title = map.next_value()?;
            }
            "name" => {
                seen.accept(16, key)?;
                self.metadata.name = map.next_value()?;
            }
            "pinned" => {
                seen.accept(32, key)?;
                self.metadata.pinned = map.next_value()?;
            }
            "session_id" => {
                seen.accept(64, key)?;
                self.resume.session_id = map.next_value()?;
            }
            "workdir" => {
                seen.accept(128, key)?;
                self.resume.workdir = map.next_value()?;
            }
            "model" => {
                seen.accept(256, key)?;
                self.resume.model = map.next_value()?;
            }
            "config" => {
                seen.accept(512, key)?;
                self.resume.config = map.next_value()?;
            }
            "mode" => {
                seen.accept(1024, key)?;
                self.resume.mode = map.next_value()?;
            }
            "pending_history" => {
                seen.accept(2048, key)?;
                self.resume.pending_history = map.next_value()?;
            }
            _ => self.read_body_field(seen, key, map)?,
        }
        Ok(())
    }
    /// Transcript and composer fields; unknown fields are skipped.
    fn read_body_field<'de, A: MapAccess<'de>>(
        &mut self,
        seen: &mut Fields,
        key: &str,
        map: &mut A,
    ) -> Result<(), A::Error> {
        match key {
            "draft" => {
                seen.accept(4096, key)?;
                self.composer.draft = map.next_value()?;
            }
            "updated" => {
                seen.accept(8192, key)?;
                self.metadata.updated = map.next_value()?;
            }
            "entries" => {
                seen.accept(16384, key)?;
                self.transcript.entries = map.next_value::<Entries>()?.0;
            }
            "queue" => {
                seen.accept(32768, key)?;
                self.composer.queue = map.next_value::<Prompts>()?.0;
            }
            "attachments" => {
                seen.accept(65536, key)?;
                self.composer.attachments = map.next_value::<Attachments>()?.0;
            }
            "times" => {
                seen.accept(131072, key)?;
                self.transcript.times = map.next_value::<Times>()?.0;
            }
            _ => {
                map.next_value::<serde::de::IgnoredAny>()?;
            }
        }
        Ok(())
    }
}

/// Preflight all bytes before allocating SDK-owned records or their vectors.
pub(super) fn decode(
    bytes: &[u8],
    expected_id: &str,
) -> Result<LegacyChat, nocterm_ai::history::HistoryCodecError> {
    use nocterm_ai::history::{HistoryCodecError, MAX_FILE_BYTES, structural_bytes};
    if uuid::Uuid::parse_str(expected_id).is_err() {
        return Err(HistoryCodecError::InvalidId(expected_id.into()));
    }
    if bytes.len() > MAX_FILE_BYTES as usize {
        return Err(HistoryCodecError::EncodedLimit);
    }
    structural_bytes(bytes)?;
    let chat: LegacyChat = serde_json::from_slice(bytes)?;
    if chat.metadata.version != 1 {
        return Err(HistoryCodecError::UnsupportedVersion(chat.metadata.version));
    }
    if chat.metadata.id != expected_id {
        return Err(HistoryCodecError::IdentityMismatch);
    }
    Ok(chat)
}

/// Historical field order/omission rules. This raw grammar serializer is not a
/// publication API: an untouched imported document must keep its physical bytes.
impl Serialize for LegacyChat {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("version", &self.metadata.version)?;
        map.serialize_entry("id", &self.metadata.id)?;
        map.serialize_entry("agent_id", &self.metadata.agent_id)?;
        if let Some(value) = &self.metadata.title {
            map.serialize_entry("title", value)?;
        }
        if let Some(value) = &self.metadata.name {
            map.serialize_entry("name", value)?;
        }
        if self.metadata.pinned {
            map.serialize_entry("pinned", &true)?;
        }
        if let Some(value) = &self.resume.session_id {
            map.serialize_entry("session_id", value)?;
        }
        if let Some(value) = &self.resume.workdir {
            map.serialize_entry("workdir", value)?;
        }
        if let Some(value) = &self.resume.model {
            map.serialize_entry("model", value)?;
        }
        if !self.resume.config.is_empty() {
            map.serialize_entry("config", &self.resume.config)?;
        }
        if let Some(value) = &self.resume.mode {
            map.serialize_entry("mode", value)?;
        }
        if self.resume.pending_history {
            map.serialize_entry("pending_history", &true)?;
        }
        if let Some(value) = &self.composer.draft {
            map.serialize_entry("draft", value)?;
        }
        map.serialize_entry("updated", &self.metadata.updated)?;
        map.serialize_entry("entries", &self.transcript.entries)?;
        if !self.composer.queue.is_empty() {
            map.serialize_entry("queue", &self.composer.queue)?;
        }
        if !self.composer.attachments.is_empty() {
            map.serialize_entry("attachments", &self.composer.attachments)?;
        }
        if !self.transcript.times.is_empty() {
            map.serialize_entry("times", &self.transcript.times)?;
        }
        map.end()
    }
}

/// Moving conversion keeps original title/name/input and presentation evidence,
/// while deliberately giving restored rows no executable bridge claims.
impl LegacyChat {
    #[cfg(test)]
    pub(super) fn into_owned(
        self,
        authority: &nocterm_ai::model::retention::RetentionAuthority,
    ) -> Result<nocterm_ai::history::SavedChat, nocterm_ai::history::HistoryCodecError> {
        self.into_owned_ordered(authority).map(|(chat, _)| chat)
    }
    pub(super) fn into_owned_ordered(
        self,
        authority: &nocterm_ai::model::retention::RetentionAuthority,
    ) -> Result<
        (nocterm_ai::history::SavedChat, super::borrowed::Grammar),
        nocterm_ai::history::HistoryCodecError,
    > {
        let mut grammar = super::borrowed::Grammar::new(authority);
        use nocterm_ai::history::{
            ChatMetadata, ComposerData, ResumeDescriptor, SavedChat, TranscriptData,
        };
        let metadata = self.metadata;
        let resume = self.resume;
        let transcript = TranscriptData {
            entries: owned_entries(self.transcript.entries, authority, &mut grammar)?,
            times: self.transcript.times,
        };
        let composer = ComposerData {
            draft: self.composer.draft,
            attachments: self.composer.attachments,
            queue: owned_queue(self.composer.queue, authority)?,
        };
        SavedChat::from_parts(
            authority,
            metadata.id,
            ChatMetadata {
                agent_id: metadata.agent_id,
                title: metadata.title,
                name: metadata.name,
                pinned: metadata.pinned,
                updated: metadata.updated,
                model: resume.model,
            },
            ResumeDescriptor {
                session_id: resume.session_id,
                workdir: resume.workdir,
                config: resume.config,
                mode: resume.mode,
                pending_history: resume.pending_history,
            },
            transcript,
            composer,
        )
        .map(|chat| (chat, grammar))
    }
}

/// Restored rows; tool rows record their original grammar order.
fn owned_entries(
    entries: Vec<LegacyEntry>,
    authority: &nocterm_ai::model::retention::RetentionAuthority,
    grammar: &mut super::borrowed::Grammar,
) -> Result<Vec<nocterm_ai::thread::Entry>, nocterm_ai::model::diagnostic::ModelError> {
    use nocterm_ai::thread::Entry;
    entries
        .into_iter()
        .map(|row| {
            Ok(match row {
                LegacyEntry::User(values) => Entry::User(
                    values
                        .into_iter()
                        .map(|block| block.into_owned(authority))
                        .collect::<Result<_, _>>()?,
                ),
                LegacyEntry::Agent(value) => Entry::Agent(value),
                LegacyEntry::Thought(value) => Entry::Thought(value),
                LegacyEntry::Content(value) => Entry::Content(value.into_owned(authority)?),
                LegacyEntry::Tool(value) => {
                    let (row, order) = super::tools::moving_ordered(authority, value)?;
                    grammar.record(order).map_err(|error| {
                        nocterm_ai::model::diagnostic::ModelError::Retention(
                            nocterm_core::SharedError::from_error(error),
                        )
                    })?;
                    Entry::Tool(row)
                }
            })
        })
        .collect()
}
fn owned_queue(
    queue: Vec<LegacyPrompt>,
    authority: &nocterm_ai::model::retention::RetentionAuthority,
) -> Result<Vec<nocterm_ai::history::SavedPrompt>, nocterm_ai::model::diagnostic::ModelError> {
    queue
        .into_iter()
        .map(|prompt| {
            Ok(nocterm_ai::history::SavedPrompt {
                id: prompt.id,
                text: prompt.text,
                images: prompt
                    .images
                    .into_iter()
                    .map(|block| block.into_owned(authority))
                    .collect::<Result<_, _>>()?,
                attachments: prompt.attachments,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_ai::{
        model::content::{Content, MediaKind},
        thread::Entry,
    };
    use serde_json::json;

    #[test]
    fn imported_rows_keep_literal_metadata_and_full_queued_media_without_live_authority() {
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap();
        let id = "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c";
        let encoded = "e".repeat(600 * 1024);
        let bytes = serde_json::to_vec(&json!({
            "version":1,"id":id,"agent_id":"provider","updated":17,
            "entries":[{"kind":"tool","value":{
                "toolCallId":"original","title":"historical title","name":"unregistered.tool",
                "kind":"execute","status":"completed","rawInput":{"literal":true},
                "_meta":{"future":{"keep":null},"nocterm/tool-display":{
                    "version":2,"tool":"historical","arguments":null,
                    "destination":null,"redacted":false,"outcome":{"Ok":null}
                }}
            }}],
            "queue":[{"id":4,"text":"queued","images":[{
                "type":"image","data":encoded,"mimeType":"image/png"
            }]}]
        }))
        .unwrap();
        let owned = decode(&bytes, id).unwrap().into_owned(&authority).unwrap();
        let Entry::Tool(tool) = &owned.transcript().entries[0] else {
            panic!("tool row")
        };
        assert_eq!(tool.title, "historical title");
        assert_eq!(tool.name.as_deref(), Some("unregistered.tool"));
        assert_eq!(
            tool.arguments.as_ref().map(|value| value.as_value()),
            Some(&json!({"literal":true}))
        );
        assert!(tool.claims.consistent_identity().is_none());
        assert_eq!(
            tool.extensions.as_ref().unwrap().get("future"),
            Some(&json!({"keep":null}))
        );
        assert!(tool.client_display.is_some());
        let Content::Media(media) = &owned.composer().queue[0].images[0] else {
            panic!("queued media")
        };
        assert_eq!(media.kind, MediaKind::Image);
        assert_eq!(media.encoded, encoded);
        // This is moving conversion coverage, not a dispatch/publication proof.
    }
}
