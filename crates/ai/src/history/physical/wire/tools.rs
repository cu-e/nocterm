//! Tool JSON presence and field order remain identical to the neutral DTO wire.
use super::super::{DiffSlot, LocationSlot, OutputSlot, ToolSlot};
use super::{Read, Tag, content::Content};
use serde::{
    Serialize, Serializer,
    ser::{SerializeSeq, SerializeStruct},
};
pub(super) struct Tool<'a>(pub &'a ToolSlot);
impl Serialize for Tool<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        fields!(s,"ToolRecord";"id"=Read(self.0.id()),"title"=Read(self.0.title()),"name"=self.0.name().map(Read),
            "kind"=self.0.kind(),"status"=self.0.status(),"output"=Outputs(self.0.output()),"locations"=Locations(self.0.locations()),
            "arguments"=Presence(self.0.arguments()),"result"=Presence(self.0.result()),"extensions"=self.0.extensions().map(Read),"client_display"=self.0.client_display().map(Read))
    }
}
struct Presence<'a>(Option<super::super::PayloadGuard<'a, crate::model::json::OwnedJson>>);
impl Serialize for Presence<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            None => fields!(s,"JsonPresence";"presence"="absent"),
            Some(value) => fields!(s,"JsonPresence";"presence"="supplied","value"=**value),
        }
    }
}
struct Outputs<'a>(&'a [OutputSlot]);
impl Serialize for Outputs<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&Output(value))?;
        }
        seq.end()
    }
}
pub(super) struct Output<'a>(pub &'a OutputSlot);
impl Serialize for Output<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            OutputSlot::Content {
                content,
                extensions,
            } => Tag {
                kind: "content",
                value: OutputContent {
                    content: Content {
                        value: content,
                        omit_images: false,
                    },
                    extensions: extensions.as_ref().map(|v| Read(v.read())),
                },
            }
            .serialize(s),
            OutputSlot::Diff(value) => Tag {
                kind: "file_diff",
                value: Diff(value),
            }
            .serialize(s),
            OutputSlot::Terminal { id, extensions } => Tag {
                kind: "terminal_reference",
                value: Terminal {
                    id: Read(id.id()),
                    extensions: extensions.as_ref().map(|v| Read(v.read())),
                },
            }
            .serialize(s),
        }
    }
}
#[derive(Serialize)]
struct OutputContent<'a> {
    content: Content<'a>,
    extensions: Option<Read<super::super::PayloadGuard<'a, crate::model::extensions::Extensions>>>,
}
#[derive(Serialize)]
struct Terminal<'a> {
    id: Read<super::super::PayloadGuard<'a, crate::model::ids::ProviderTerminalId>>,
    extensions: Option<Read<super::super::PayloadGuard<'a, crate::model::extensions::Extensions>>>,
}
struct Diff<'a>(&'a DiffSlot);
impl Serialize for Diff<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        fields!(s,"FileDiff";"path"=Read(self.0.path()),"previous"=self.0.previous().map(Read),"replacement"=Read(self.0.replacement()),"extensions"=self.0.extensions().map(Read))
    }
}
struct Locations<'a>(&'a [LocationSlot]);
impl Serialize for Locations<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&Location(value))?;
        }
        seq.end()
    }
}
struct Location<'a>(&'a LocationSlot);
impl Serialize for Location<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        fields!(s,"Location";"path"=Read(self.0.path()),"line"=self.0.line(),"extensions"=self.0.extensions().map(Read))
    }
}
