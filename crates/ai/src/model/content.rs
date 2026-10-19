//! Conversation content. Saved transcripts store these records, so their JSON shape
//! (names, field order, omitted defaults and lenient decoding) is a compatibility contract.
use super::extensions::Extensions;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, VecSkipError, serde_as, skip_serializing_none};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text(Text),
    Image(Image),
    Audio(Audio),
    #[serde(rename = "resource_link")]
    ResourceReference(ResourceReference),
    #[serde(rename = "resource")]
    EmbeddedResource(EmbeddedResource),
}
impl Content {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(Text::new(text))
    }
}
impl From<&str> for Content {
    fn from(value: &str) -> Self {
        Self::text(value)
    }
}
impl From<String> for Content {
    fn from(value: String) -> Self {
        Self::text(value)
    }
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Text {
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub annotations: Option<Annotations>,
    pub text: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}
impl Text {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            annotations: None,
            text: text.into(),
            meta: None,
        }
    }
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub annotations: Option<Annotations>,
    pub data: String,
    pub mime_type: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub uri: Option<String>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}
impl Image {
    pub fn new(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self {
            annotations: None,
            data: data.into(),
            mime_type: mime_type.into(),
            uri: None,
            meta: None,
        }
    }
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Audio {
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub annotations: Option<Annotations>,
    pub data: String,
    pub mime_type: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceReference {
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub annotations: Option<Annotations>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub description: Option<String>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub mime_type: Option<String>,
    pub name: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub size: Option<i64>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub title: Option<String>,
    pub uri: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EmbeddedResource {
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub annotations: Option<Annotations>,
    pub resource: ResourceBody,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

/// Text is tried before binary data, as in the saved-file grammar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResourceBody {
    Text(ResourceText),
    Blob(ResourceBlob),
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceText {
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub mime_type: Option<String>,
    pub text: String,
    pub uri: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBlob {
    pub blob: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub mime_type: Option<String>,
    pub uri: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotations {
    #[serde_as(deserialize_as = "DefaultOnError<Option<VecSkipError<_>>>")]
    #[serde(default)]
    pub audience: Option<Vec<Audience>>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub priority: Option<f64>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Audience {
    Assistant,
    User,
}
