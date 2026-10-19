//! Version-one content grammar over borrowed neutral facts, without SDK allocation.
use nocterm_ai::{
    history::CompatibilityReason,
    model::{content::*, extensions::Extensions},
};
use serde::{
    Serialize,
    ser::{SerializeMap, SerializeSeq},
};

pub(super) fn check(value: &Content) -> Result<(), CompatibilityReason> {
    if matches!(
        value,
        Content::Media(Media {
            kind: MediaKind::Audio,
            uri: Some(_),
            ..
        })
    ) {
        return Err(CompatibilityReason::ContentField);
    }
    Ok(())
}
pub(super) struct Borrowed<'a>(pub(super) &'a Content);
impl Serialize for Borrowed<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            Content::Text(value) => {
                map.serialize_entry("type", "text")?;
                annotation(&mut map, &value.annotations)?;
                map.serialize_entry("text", &value.text)?;
                metadata(&mut map, &value.extensions)?;
            }
            Content::Media(value) => {
                map.serialize_entry(
                    "type",
                    match value.kind {
                        MediaKind::Image => "image",
                        MediaKind::Audio => "audio",
                    },
                )?;
                annotation(&mut map, &value.annotations)?;
                map.serialize_entry("data", &value.encoded)?;
                map.serialize_entry("mimeType", &value.mime)?;
                if matches!(value.kind, MediaKind::Image) {
                    optional(&mut map, "uri", &value.uri)?;
                } else if value.uri.is_some() {
                    return Err(serde::ser::Error::custom(
                        "legacy audio has no source URI field",
                    ));
                }
                metadata(&mut map, &value.extensions)?;
            }
            Content::ResourceReference(value) => {
                map.serialize_entry("type", "resource_link")?;
                annotation(&mut map, &value.annotations)?;
                optional(&mut map, "description", &value.description)?;
                optional(&mut map, "mimeType", &value.mime)?;
                map.serialize_entry("name", &value.name)?;
                optional(&mut map, "size", &value.reported_size)?;
                optional(&mut map, "title", &value.title)?;
                map.serialize_entry("uri", &value.uri)?;
                metadata(&mut map, &value.extensions)?;
            }
            Content::EmbeddedResource(value) => {
                map.serialize_entry("type", "resource")?;
                annotation(&mut map, &value.annotations)?;
                map.serialize_entry("resource", &Body(&value.body))?;
                metadata(&mut map, &value.extensions)?;
            }
        }
        map.end()
    }
}
struct Body<'a>(&'a ResourceBody);
impl Serialize for Body<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        match self.0 {
            ResourceBody::Text(value) => {
                optional(&mut map, "mimeType", &value.mime)?;
                map.serialize_entry("text", &value.text)?;
                map.serialize_entry("uri", &value.uri)?;
                metadata(&mut map, &value.extensions)?;
            }
            ResourceBody::Encoded(value) => {
                map.serialize_entry("blob", &value.encoded)?;
                optional(&mut map, "mimeType", &value.mime)?;
                map.serialize_entry("uri", &value.uri)?;
                metadata(&mut map, &value.extensions)?;
            }
        }
        map.end()
    }
}
fn annotation<M: SerializeMap>(map: &mut M, value: &Option<Annotations>) -> Result<(), M::Error> {
    if let Some(value) = value {
        map.serialize_entry("annotations", &Annotation(value))?;
    }
    Ok(())
}
struct Annotation<'a>(&'a Annotations);
impl Serialize for Annotation<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if let Some(audience) = &self.0.audience {
            map.serialize_entry("audience", &Audiences(audience))?;
        }
        optional(&mut map, "lastModified", &self.0.modified)?;
        optional(&mut map, "priority", &self.0.priority)?;
        metadata(&mut map, &self.0.extensions)?;
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
pub(super) fn metadata<M: SerializeMap>(
    map: &mut M,
    value: &Option<Extensions>,
) -> Result<(), M::Error> {
    if let Some(value) = value {
        map.serialize_entry("_meta", value)?;
    }
    Ok(())
}
pub(super) fn optional<M: SerializeMap, T: Serialize>(
    map: &mut M,
    key: &'static str,
    value: &Option<T>,
) -> Result<(), M::Error> {
    if let Some(value) = value {
        map.serialize_entry(key, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol_schema::v1 as wire;
    use serde_json::json;

    #[test]
    fn borrowed_legacy_content_matches_private_sdk_grammar_after_moving_import() {
        let authority = nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap();
        let fixtures = [
            json!({"type":"text","text":"literal λ","annotations":{"audience":["user","assistant"],"lastModified":"old","priority":0.5,"_meta":{"null":null}},"_meta":{"nested":{"x":1}}}),
            json!({"type":"image","data":"bytes","mimeType":"image/png","uri":"original"}),
            json!({"type":"audio","data":"full","mimeType":"audio/ogg"}),
            json!({"type":"resource_link","name":"name","uri":"uri","description":"description","size":12,"title":"title","mimeType":"text/plain"}),
            json!({"type":"resource","resource":{"mimeType":"text/plain","text":"body","uri":"uri","_meta":{"keep":true}}}),
            json!({"type":"resource","resource":{"blob":"encoded","uri":"uri"}}),
        ];
        for fixture in fixtures {
            let wire: wire::ContentBlock = serde_json::from_value(fixture).unwrap();
            let expected = serde_json::to_vec(&wire).unwrap();
            let owned = super::super::content::moving(&authority, wire).unwrap();
            check(&owned).unwrap();
            let actual = serde_json::to_vec(&Borrowed(&owned)).unwrap();
            assert_eq!(actual, expected);
            let measured = nocterm_ai::history::measure_json(&Borrowed(&owned)).unwrap();
            assert_eq!(measured.encoded_bytes, actual.len());
            assert_eq!(
                measured.structural_bytes,
                nocterm_ai::history::structural_bytes(&actual).unwrap()
            );
        }
    }
}
