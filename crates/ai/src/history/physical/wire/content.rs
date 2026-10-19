//! Managed content borrows preserve the original selective transcript image rule.
use super::super::{AnnotationSlot, ContentSlot, ResourceSlot};
use super::{Read, Tag};
use serde::{
    Serialize, Serializer,
    ser::{SerializeSeq, SerializeStruct},
};

pub(super) struct Content<'a> {
    pub value: &'a ContentSlot,
    pub omit_images: bool,
}
/// A User row: its items are independent bundles, serialized in order.
pub(super) struct UserContents<'a>(pub &'a super::super::UserSlot, pub bool);
impl Serialize for UserContents<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for value in self.0.contents() {
            seq.serialize_element(&Content {
                value,
                omit_images: self.1,
            })?;
        }
        seq.end()
    }
}
impl Serialize for Content<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if self.omit_images
            && let ContentSlot::Media(media) = self.value
            && media.kind() == crate::model::content::MediaKind::Image
            && media.encoded().len() > crate::history::MAX_SAVED_IMAGE_BYTES
        {
            #[derive(Serialize)]
            struct Omitted {
                text: &'static str,
                annotations: Option<()>,
                extensions: Option<()>,
            }
            return Tag {
                kind: "text",
                value: Omitted {
                    text: "[Image not kept in history]",
                    annotations: None,
                    extensions: None,
                },
            }
            .serialize(s);
        }
        let kind = match self.value {
            ContentSlot::Text(_) => "text",
            ContentSlot::Media(_) => "media",
            ContentSlot::Reference(_) => "resource_reference",
            ContentSlot::Embedded(_) => "embedded_resource",
        };
        Tag {
            kind,
            value: Body(self.value),
        }
        .serialize(s)
    }
}
struct Body<'a>(&'a ContentSlot);
impl Serialize for Body<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let annotations = self.0.annotations().map(Annotation);
        let extensions = self.0.extensions().map(Read);
        match self.0 {
            ContentSlot::Text(v) => {
                fields!(s,"Text";"text"=Read(v.text()),"annotations"=annotations,"extensions"=extensions)
            }
            ContentSlot::Media(v) => {
                fields!(s,"Media";"kind"=v.kind(),"encoded"=Read(v.encoded()),"mime"=Read(v.mime()),
                "uri"=v.uri().map(Read),"annotations"=annotations,"extensions"=extensions)
            }
            ContentSlot::Reference(v) => {
                fields!(s,"ResourceReference";"uri"=Read(v.uri()),"name"=Read(v.name()),"title"=v.title().map(Read),
                "description"=v.description().map(Read),"mime"=v.mime().map(Read),"reported_size"=v.reported_size(),"annotations"=annotations,"extensions"=extensions)
            }
            ContentSlot::Embedded(v) => {
                fields!(s,"EmbeddedResource";"body"=Resource(v.body()),"annotations"=annotations,"extensions"=extensions)
            }
        }
    }
}
struct Resource<'a>(&'a ResourceSlot);
impl Serialize for Resource<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            ResourceSlot::Text(v) => Tag {
                kind: "text",
                value: ResourceBody::Text(v),
            }
            .serialize(s),
            ResourceSlot::Encoded(v) => Tag {
                kind: "encoded",
                value: ResourceBody::Encoded(v),
            }
            .serialize(s),
        }
    }
}
enum ResourceBody<'a> {
    Text(&'a super::super::ResourceTextSlot),
    Encoded(&'a super::super::ResourceBlobSlot),
}
impl Serialize for ResourceBody<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(v) => {
                fields!(s,"ResourceText";"uri"=Read(v.uri()),"text"=Read(v.text()),"mime"=v.mime().map(Read),"extensions"=v.extensions().map(Read))
            }
            Self::Encoded(v) => {
                fields!(s,"ResourceBlob";"uri"=Read(v.uri()),"encoded"=Read(v.encoded()),"mime"=v.mime().map(Read),"extensions"=v.extensions().map(Read))
            }
        }
    }
}
struct Annotation<'a>(&'a AnnotationSlot);
impl Serialize for Annotation<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        fields!(s,"Annotations";"audience"=self.0.audience(),"modified"=self.0.modified().map(Read),"priority"=self.0.priority(),"extensions"=self.0.extensions().map(Read))
    }
}
