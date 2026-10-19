use super::extensions::Extensions;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Content {
    Text(Text),
    Media(Media),
    ResourceReference(ResourceReference),
    EmbeddedResource(EmbeddedResource),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Text {
    pub text: String,
    pub annotations: Option<Annotations>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Audio,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Media {
    pub kind: MediaKind,
    pub encoded: String,
    pub mime: String,
    pub uri: Option<String>,
    pub annotations: Option<Annotations>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResourceReference {
    pub uri: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime: Option<String>,
    pub reported_size: Option<i64>,
    pub annotations: Option<Annotations>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EmbeddedResource {
    pub body: ResourceBody,
    pub annotations: Option<Annotations>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ResourceBody {
    Text(ResourceText),
    Encoded(ResourceBlob),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResourceText {
    pub uri: String,
    pub text: String,
    pub mime: Option<String>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResourceBlob {
    pub uri: String,
    pub encoded: String,
    pub mime: Option<String>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Annotations {
    pub audience: Option<Vec<Audience>>,
    pub modified: Option<String>,
    pub priority: Option<f64>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    User,
    Assistant,
}
