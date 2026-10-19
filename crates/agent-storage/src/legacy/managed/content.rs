//! Version-one content grammar over borrowed neutral facts, without SDK allocation.
use super::{Read, metadata, optional};
use nocterm_ai::{
    history::{CompatibilityReason, physical::*},
    model::content::{Audience, MediaKind},
};
use serde::{
    Serialize,
    ser::{SerializeMap, SerializeSeq},
};

pub(super) fn check(value: &ContentSlot) -> Result<(), CompatibilityReason> {
    if matches!(
        value,
        ContentSlot::Media(value) if value.kind() == MediaKind::Audio && value.uri().is_some()
    ) {
        return Err(CompatibilityReason::ContentField);
    }
    Ok(())
}
pub(super) struct Borrowed<'a>(pub(super) &'a ContentSlot);
impl Serialize for Borrowed<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            ContentSlot::Text(value) => {
                map.serialize_entry("type", "text")?;
                annotation(&mut map, self.0.annotations())?;
                map.serialize_entry("text", &Read(value.text()))?;
                metadata(&mut map, self.0.extensions())?;
            }
            ContentSlot::Media(value) => {
                map.serialize_entry(
                    "type",
                    match value.kind() {
                        MediaKind::Image => "image",
                        MediaKind::Audio => "audio",
                    },
                )?;
                annotation(&mut map, self.0.annotations())?;
                map.serialize_entry("data", &Read(value.encoded()))?;
                map.serialize_entry("mimeType", &Read(value.mime()))?;
                if matches!(value.kind(), MediaKind::Image) {
                    optional(&mut map, "uri", &value.uri().map(Read))?;
                } else if value.uri().is_some() {
                    return Err(serde::ser::Error::custom(
                        "legacy audio has no source URI field",
                    ));
                }
                metadata(&mut map, self.0.extensions())?;
            }
            ContentSlot::Reference(value) => {
                map.serialize_entry("type", "resource_link")?;
                annotation(&mut map, self.0.annotations())?;
                optional(&mut map, "description", &value.description().map(Read))?;
                optional(&mut map, "mimeType", &value.mime().map(Read))?;
                map.serialize_entry("name", &Read(value.name()))?;
                optional(&mut map, "size", &value.reported_size())?;
                optional(&mut map, "title", &value.title().map(Read))?;
                map.serialize_entry("uri", &Read(value.uri()))?;
                metadata(&mut map, self.0.extensions())?;
            }
            ContentSlot::Embedded(value) => {
                map.serialize_entry("type", "resource")?;
                annotation(&mut map, self.0.annotations())?;
                map.serialize_entry("resource", &Body(value.body()))?;
                metadata(&mut map, self.0.extensions())?;
            }
        }
        map.end()
    }
}
struct Body<'a>(&'a ResourceSlot);
impl Serialize for Body<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            ResourceSlot::Text(value) => {
                optional(&mut map, "mimeType", &value.mime().map(Read))?;
                map.serialize_entry("text", &Read(value.text()))?;
                map.serialize_entry("uri", &Read(value.uri()))?;
                metadata(&mut map, value.extensions())?;
            }
            ResourceSlot::Encoded(value) => {
                map.serialize_entry("blob", &Read(value.encoded()))?;
                optional(&mut map, "mimeType", &value.mime().map(Read))?;
                map.serialize_entry("uri", &Read(value.uri()))?;
                metadata(&mut map, value.extensions())?;
            }
        }
        map.end()
    }
}
fn annotation<M: SerializeMap>(
    map: &mut M,
    value: Option<&AnnotationSlot>,
) -> Result<(), M::Error> {
    if let Some(value) = value {
        map.serialize_entry("annotations", &Annotation(value))?;
    }
    Ok(())
}
struct Annotation<'a>(&'a AnnotationSlot);
impl Serialize for Annotation<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if let Some(audience) = self.0.audience() {
            map.serialize_entry("audience", &Audiences(audience))?;
        }
        optional(&mut map, "lastModified", &self.0.modified().map(Read))?;
        optional(&mut map, "priority", &self.0.priority())?;
        metadata(&mut map, self.0.extensions())?;
        map.end()
    }
}
struct Audiences<'a>(&'a [Audience]);
impl Serialize for Audiences<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for role in self.0 {
            seq.serialize_element(match role {
                Audience::User => "user",
                Audience::Assistant => "assistant",
            })?;
        }
        seq.end()
    }
}
