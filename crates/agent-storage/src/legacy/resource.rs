//! Consuming historical Text-first/Blob-second resource grammar.
//!
//! Branch validity is retained independently: fields ignored by the Text branch
//! cannot make it fail, while a malformed Text branch may still select Blob.
//! The selected payload is moved, never borrowed from a retained untagged tree.
use nocterm_ai::model::{
    content::{ResourceBlob, ResourceBody, ResourceText},
    diagnostic::ModelError,
    extensions::Extensions,
};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Default)]
pub(super) struct Resource {
    uri: Option<String>,
    mime: Option<String>,
    meta: Option<serde_json::Map<String, Value>>,
    text: Option<String>,
    blob: Option<String>,
    seen: u8,
    invalid: u8,
}
impl Resource {
    fn field(&mut self, bit: u8, branches: u8) {
        if self.seen & bit != 0 {
            self.invalid |= branches;
        }
        self.seen |= bit;
    }
    fn string(value: Value) -> Option<String> {
        match value {
            Value::String(text) => Some(text),
            _ => None,
        }
    }
    fn metadata(value: Value) -> Option<serde_json::Map<String, Value>> {
        match value {
            Value::Object(meta) => Some(meta),
            _ => None,
        }
    }
    fn valid(&self) -> bool {
        self.uri.is_some()
            && ((self.invalid & 1 == 0 && self.text.is_some())
                || (self.invalid & 2 == 0 && self.blob.is_some()))
    }
    pub(super) fn into_owned(
        self,
        authority: &nocterm_ai::model::retention::RetentionAuthority,
    ) -> Result<ResourceBody, ModelError> {
        let extensions = self
            .meta
            .map(|meta| Extensions::checked(authority, meta))
            .transpose()?;
        let uri = self
            .uri
            .ok_or(ModelError::Unsupported("legacy resource uri"))?;
        if self.invalid & 1 == 0
            && let Some(text) = self.text
        {
            return Ok(ResourceBody::Text(ResourceText {
                uri,
                text,
                mime: self.mime,
                extensions,
            }));
        }
        if self.invalid & 2 == 0
            && let Some(encoded) = self.blob
        {
            return Ok(ResourceBody::Encoded(ResourceBlob {
                uri,
                encoded,
                mime: self.mime,
                extensions,
            }));
        }
        Err(ModelError::Unsupported("legacy resource branch"))
    }
}
impl<'de> Deserialize<'de> for Resource {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        decoder.deserialize_any(Contents)
    }
}
struct Contents;
impl<'de> Visitor<'de> for Contents {
    type Value = Resource;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("legacy text or blob resource contents")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Resource, A::Error> {
        let mut row = Resource::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "uri" => {
                    row.field(1, 3);
                    row.uri = Resource::string(map.next_value()?);
                    if row.uri.is_none() {
                        row.invalid |= 3;
                    }
                }
                "mimeType" => {
                    row.field(2, 3);
                    // Original optional DefaultOnError maps null/invalid to None.
                    row.mime = Resource::string(map.next_value()?);
                }
                "_meta" => {
                    row.field(4, 3);
                    row.meta = Resource::metadata(map.next_value()?);
                }
                "text" => {
                    row.field(8, 1);
                    row.text = Resource::string(map.next_value()?);
                    if row.text.is_none() {
                        row.invalid |= 1;
                    }
                }
                "blob" => {
                    row.field(16, 2);
                    row.blob = Resource::string(map.next_value()?);
                    if row.blob.is_none() {
                        row.invalid |= 2;
                    }
                }
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        if row.valid() {
            Ok(row)
        } else {
            Err(serde::de::Error::custom(
                "data did not match legacy resource contents",
            ))
        }
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Resource, A::Error> {
        // Original derived struct declaration order differs per branch.
        // Text: mimeType,text,uri,_meta; Blob: blob,mimeType,uri,_meta.
        let first: Value = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
        let second: Value = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
        let third: Value = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
        let meta = seq.next_element::<Value>()?.and_then(Resource::metadata);
        if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
            return Err(serde::de::Error::invalid_length(5, &self));
        }
        let uri = Resource::string(third);
        let row = match second {
            Value::String(text) => Resource {
                uri,
                mime: Resource::string(first),
                meta,
                text: Some(text),
                ..Resource::default()
            },
            _ => Resource {
                uri,
                blob: Resource::string(first),
                meta,
                invalid: 1,
                ..Resource::default()
            },
        };
        if row.valid() {
            Ok(row)
        } else {
            Err(serde::de::Error::custom(
                "data did not match legacy resource contents",
            ))
        }
    }
}

impl serde::Serialize for Resource {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut fields = serializer.serialize_map(None)?;
        if self.invalid & 1 == 0
            && let Some(text) = &self.text
        {
            if let Some(mime) = &self.mime {
                fields.serialize_entry("mimeType", mime)?;
            }
            fields.serialize_entry("text", text)?;
        } else if self.invalid & 2 == 0
            && let Some(blob) = &self.blob
        {
            fields.serialize_entry("blob", blob)?;
            if let Some(mime) = &self.mime {
                fields.serialize_entry("mimeType", mime)?;
            }
        } else {
            return Err(serde::ser::Error::custom("invalid legacy resource branch"));
        }
        fields.serialize_entry(
            "uri",
            self.uri
                .as_ref()
                .ok_or_else(|| serde::ser::Error::custom("missing resource uri"))?,
        )?;
        if let Some(meta) = &self.meta {
            fields.serialize_entry("_meta", meta)?;
        }
        fields.end()
    }
}
