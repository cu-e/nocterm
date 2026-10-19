//! ACP content borrows the actual prompt bundles, with original wire omission.
use nocterm_ai::{
    history::physical::{AnnotationSlot, ContentSlot, ResourceSlot},
    model::content::MediaKind,
};
use serde::{Serialize, Serializer, ser::SerializeMap};
use std::ops::Deref;
pub(super) struct Read<T>(pub T);
impl<T: Deref> Serialize for Read<T>
where
    T::Target: Serialize,
{
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.deref().serialize(s)
    }
}
fn optional<M: SerializeMap, V: Serialize>(
    map: &mut M,
    key: &'static str,
    value: Option<V>,
) -> Result<(), M::Error> {
    if let Some(value) = value {
        map.serialize_entry(key, &value)?;
    }
    Ok(())
}
struct Annotation<'a>(&'a AnnotationSlot);
impl Serialize for Annotation<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        optional(&mut map, "audience", self.0.audience())?;
        optional(&mut map, "lastModified", self.0.modified().map(Read))?;
        optional(&mut map, "priority", self.0.priority())?;
        optional(&mut map, "_meta", self.0.extensions().map(Read))?;
        map.end()
    }
}
pub(super) struct Content<'a>(pub &'a ContentSlot);
impl Serialize for Content<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        let kind = match self.0 {
            ContentSlot::Text(_) => "text",
            ContentSlot::Media(value) => match value.kind() {
                MediaKind::Image => "image",
                MediaKind::Audio => "audio",
            },
            ContentSlot::Reference(_) => "resource_link",
            ContentSlot::Embedded(_) => "resource",
        };
        map.serialize_entry("type", kind)?;
        optional(
            &mut map,
            "annotations",
            self.0.annotations().map(Annotation),
        )?;
        match self.0 {
            ContentSlot::Text(value) => map.serialize_entry("text", &Read(value.text()))?,
            ContentSlot::Media(value) => {
                if value.kind() == MediaKind::Audio && value.uri().is_some() {
                    return Err(serde::ser::Error::custom("audio source URI"));
                }
                map.serialize_entry("data", &Read(value.encoded()))?;
                map.serialize_entry("mimeType", &Read(value.mime()))?;
                if value.kind() == MediaKind::Image {
                    optional(&mut map, "uri", value.uri().map(Read))?;
                }
            }
            ContentSlot::Reference(value) => {
                map.serialize_entry("name", &Read(value.name()))?;
                map.serialize_entry("uri", &Read(value.uri()))?;
                optional(&mut map, "title", value.title().map(Read))?;
                optional(&mut map, "description", value.description().map(Read))?;
                optional(&mut map, "mimeType", value.mime().map(Read))?;
                optional(&mut map, "size", value.reported_size())?;
            }
            ContentSlot::Embedded(value) => {
                map.serialize_entry("resource", &Resource(value.body()))?
            }
        }
        optional(&mut map, "_meta", self.0.extensions().map(Read))?;
        map.end()
    }
}
struct Resource<'a>(&'a ResourceSlot);
impl Serialize for Resource<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        match self.0 {
            ResourceSlot::Text(value) => {
                map.serialize_entry("uri", &Read(value.uri()))?;
                map.serialize_entry("text", &Read(value.text()))?;
                optional(&mut map, "mimeType", value.mime().map(Read))?;
                optional(&mut map, "_meta", value.extensions().map(Read))?;
            }
            ResourceSlot::Encoded(value) => {
                map.serialize_entry("uri", &Read(value.uri()))?;
                map.serialize_entry("blob", &Read(value.encoded()))?;
                optional(&mut map, "mimeType", value.mime().map(Read))?;
                optional(&mut map, "_meta", value.extensions().map(Read))?;
            }
        }
        map.end()
    }
}
#[derive(Serialize)]
pub(super) struct Text<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
}
impl<'a> Text<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        Self { kind: "text", text }
    }
}
