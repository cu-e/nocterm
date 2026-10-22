//! Version-aware intake preserves legacy leniency without inventing scoped authority.
use super::{
    HistoryCodecError, SavedAttachment, SavedChat, SavedPrompt, decode, header::IncomingHeader,
};
use crate::thread::Entry;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
pub(super) struct IncomingChat {
    version: u32,
    #[serde(flatten)]
    header: IncomingHeader,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    workdir: Option<PathBuf>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    config: crate::session_config::Choices,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    pending_history: bool,
    #[serde(default)]
    draft: Option<String>,
    #[serde(deserialize_with = "decode::entries")]
    entries: Vec<Entry>,
    #[serde(default, deserialize_with = "decode::prompts")]
    queue: Vec<SavedPrompt>,
    #[serde(default, deserialize_with = "decode::attachments")]
    attachments: Vec<SavedAttachment>,
    #[serde(default, deserialize_with = "decode::times")]
    times: Vec<u64>,
}
impl IncomingChat {
    pub(super) fn finish(self) -> Result<SavedChat, HistoryCodecError> {
        Ok(SavedChat {
            version: self.version,
            header: self.header.finish(self.version)?,
            session_id: self.session_id,
            workdir: self.workdir,
            model: self.model,
            config: self.config,
            mode: self.mode,
            pending_history: self.pending_history,
            draft: self.draft,
            entries: self.entries,
            queue: self.queue,
            attachments: self.attachments,
            times: self.times,
            prepared: None,
            archived_draft: false,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IncomingEnvelope {
    version: u32,
    catalog: super::CatalogHeader,
    chat: IncomingChat,
}
impl IncomingEnvelope {
    fn finish(self) -> Result<SavedChat, HistoryCodecError> {
        if self.version != super::VERSION {
            return Err(HistoryCodecError::UnsupportedVersion(self.version));
        }
        let chat = self.chat.finish()?;
        if chat.version != self.version
            || self.catalog != super::CatalogHeader::from_chat(&chat, false)
        {
            return Err(HistoryCodecError::HeaderMismatch);
        }
        self.catalog.validate(&chat.header.id)?;
        Ok(chat)
    }
}
fn legacy(chat: IncomingChat) -> Result<SavedChat, HistoryCodecError> {
    let chat = chat.finish()?;
    if chat.version != super::LEGACY_VERSION {
        return Err(HistoryCodecError::MissingCatalog);
    }
    Ok(chat)
}
pub(super) fn decode_bytes(bytes: &[u8], id: &str) -> Result<SavedChat, HistoryCodecError> {
    if bytes.starts_with(super::prefix::START) {
        super::decode_catalog_header(bytes, id)?;
        serde_json::from_slice::<IncomingEnvelope>(bytes)?.finish()
    } else {
        legacy(serde_json::from_slice(bytes)?)
    }
}

/// Replay only the first key/value; this avoids buffering whole payloads to
/// choose between legacy flat records and the scoped envelope.
struct ReplayMap<A> {
    key: Option<String>,
    value: Option<u32>,
    rest: A,
}
impl<'de, A: serde::de::MapAccess<'de>> serde::de::MapAccess<'de> for ReplayMap<A> {
    type Error = A::Error;
    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        if let Some(key) = self.key.take() {
            seed.deserialize(serde::de::value::StringDeserializer::new(key))
                .map(Some)
        } else {
            self.rest.next_key_seed(seed)
        }
    }
    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, A::Error> {
        if let Some(value) = self.value.take() {
            seed.deserialize(serde::de::value::U32Deserializer::new(value))
        } else {
            self.rest.next_value_seed(seed)
        }
    }
}
pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<SavedChat, D::Error> {
    struct Record;
    impl<'de> serde::de::Visitor<'de> for Record {
        type Value = SavedChat;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a legacy chat or scoped chat envelope")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<SavedChat, A::Error> {
            use serde::de::{Error as _, value::MapAccessDeserializer};
            let key = map
                .next_key::<String>()?
                .ok_or_else(|| A::Error::custom("missing chat fields"))?;
            let version = (key == "version")
                .then(|| map.next_value::<u32>())
                .transpose()?;
            let scoped = version == Some(super::VERSION);
            let replay = MapAccessDeserializer::new(ReplayMap {
                key: Some(key),
                value: version,
                rest: map,
            });
            if scoped {
                IncomingEnvelope::deserialize(replay)?.finish()
            } else {
                legacy(IncomingChat::deserialize(replay)?)
            }
            .map_err(A::Error::custom)
        }
    }
    deserializer.deserialize_map(Record)
}
