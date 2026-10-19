//! Borrowed version-two serialization of the actual managed payload owners.
//! Serialization does not clone document values or their text/JSON backing.
use super::{ChoiceValueSlot, RowBody, RowSlot, SnapshotOwner};
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq, SerializeStruct},
};
use std::ops::Deref;
macro_rules! fields {
    ($serializer:expr, $name:literal; $($key:literal = $value:expr),* $(,)?) => {{
        let names = [$($key),*];
        let mut state = $serializer.serialize_struct($name, names.len())?;
        $(state.serialize_field($key, &$value)?;)*
        state.end()
    }};
}
mod composer;
mod content;
mod tools;
use composer::Composer;
use content::{Content, UserContents};
use tools::Tool;

pub(super) struct Read<T>(pub T);
impl<T: Deref> Serialize for Read<T>
where
    T::Target: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.deref().serialize(serializer)
    }
}
#[derive(Serialize)]
pub(super) struct Tag<T> {
    kind: &'static str,
    value: T,
}

/// Canonical v2 projection retains all live rows in memory and serializes only
/// the original bounded suffix. Queued prompts retain their full image values.
pub struct SnapshotWire<'a>(&'a SnapshotOwner);
impl<'a> SnapshotWire<'a> {
    pub fn new(snapshot: &'a SnapshotOwner) -> Self {
        Self(snapshot)
    }
}
impl Serialize for SnapshotWire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        fields!(serializer,"ChatEnvelope"; "format"=crate::history::FORMAT,
            "version"=crate::history::VERSION,"document"=Document(self.0))
    }
}
struct Document<'a>(&'a SnapshotOwner);
impl Serialize for Document<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        fields!(serializer,"Document"; "id"=Read(self.0.id()),"metadata"=Metadata(self.0.metadata()),
            "resume"=Resume(self.0.resume()),"transcript"=Transcript(self.0),"composer"=Composer(self.0.composer()))
    }
}
struct Metadata<'a>(&'a super::MetadataSlot);
impl Serialize for Metadata<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        fields!(serializer,"ChatMetadata";"agent_id"=Read(self.0.agent_id()),"title"=self.0.title().map(Read),
            "name"=self.0.name().map(Read),"pinned"=self.0.pinned(),"updated"=self.0.updated(),"model"=self.0.model().map(Read))
    }
}
struct Resume<'a>(&'a super::ResumeSlot);
impl Serialize for Resume<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        fields!(serializer,"ResumeDescriptor";"session_id"=self.0.session_id().map(Read),"workdir"=self.0.workdir().map(Read),
            "config"=Choices(self.0.config()),"mode"=self.0.mode().map(Read),"pending_history"=self.0.pending_history())
    }
}
struct Choices<'a>(&'a [super::ChoiceSlot]);
impl Serialize for Choices<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for choice in self.0 {
            match choice.value() {
                ChoiceValueSlot::Flag(value) => map.serialize_entry(&Read(choice.key()), value)?,
                ChoiceValueSlot::Value(text) => {
                    map.serialize_entry(&Read(choice.key()), &Read(text.read()))?
                }
            }
        }
        map.end()
    }
}
struct Transcript<'a>(&'a SnapshotOwner);
impl Serialize for Transcript<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let skip = self
            .0
            .rows()
            .len()
            .saturating_sub(crate::history::MAX_SAVED_ENTRIES);
        fields!(serializer,"TranscriptData";"entries"=Rows(&self.0.rows()[skip..]),"times"=self.0.times().get(skip..).unwrap_or_default())
    }
}
struct Rows<'a>(&'a [RowSlot]);
impl Serialize for Rows<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut rows = serializer.serialize_seq(Some(self.0.len()))?;
        for row in self.0 {
            rows.serialize_element(&Row(row.view(), true))?;
        }
        rows.end()
    }
}
struct Row<'a>(&'a RowBody, bool);
impl Serialize for Row<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            RowBody::Agent(text) => Tag {
                kind: "agent",
                value: Read(text.read()),
            }
            .serialize(s),
            RowBody::Thought(text) => Tag {
                kind: "thought",
                value: Read(text.read()),
            }
            .serialize(s),
            RowBody::User(values) => Tag {
                kind: "user",
                value: UserContents(values, self.1),
            }
            .serialize(s),
            RowBody::Content(value) => Tag {
                kind: "content",
                value: Content {
                    value,
                    omit_images: self.1,
                },
            }
            .serialize(s),
            RowBody::Tool(value) => Tag {
                kind: "tool",
                value: Tool(value),
            }
            .serialize(s),
        }
    }
}
pub(crate) fn retained_row(row: &RowSlot) -> Result<usize, crate::history::MeasurementError> {
    crate::history::size::retained(&Row(row.view(), false))
}
pub(crate) fn retained_prompt(
    prompt: &super::PromptSlot,
) -> Result<usize, crate::history::MeasurementError> {
    crate::history::size::retained(&composer::Prompt(prompt))
}
struct Framing<'a>(&'a SnapshotOwner);
impl Serialize for Framing<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        fields!(serializer,"Document"; "id"=Read(self.0.id()),"metadata"=Metadata(self.0.metadata()),
            "resume"=Resume(self.0.resume()),"transcript"=EmptyTranscript(self.0),"composer"=composer::Framing(self.0.composer()))
    }
}
struct EmptyTranscript<'a>(&'a SnapshotOwner);
impl Serialize for EmptyTranscript<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        fields!(serializer,"TranscriptData"; "entries"=&[] as &[()], "times"=self.0.times())
    }
}
pub(crate) fn retained_framing(
    snapshot: &SnapshotOwner,
) -> Result<usize, crate::history::MeasurementError> {
    crate::history::size::retained(&Framing(snapshot))
}

/// Full content for ephemeral presentation; transcript omission is applied only
/// by the persisted snapshot serializer.
pub(super) fn content_value(value: &super::ContentSlot) -> impl Serialize + '_ {
    Content {
        value,
        omit_images: false,
    }
}
pub(super) fn output_value(value: &super::OutputSlot) -> impl Serialize + '_ {
    tools::Output(value)
}
