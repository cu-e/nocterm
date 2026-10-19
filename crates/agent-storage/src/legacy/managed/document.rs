//! Version-one document framing over borrowed physical components.
use super::{Attachments, Prompt, Read, Row, optional};
use crate::legacy::borrowed::Grammar;
use nocterm_ai::history::{
    MAX_SAVED_ENTRIES,
    physical::{ChoiceSlot, ChoiceValueSlot, SnapshotOwner},
};
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
pub(in crate::legacy) struct Document<'a>(
    pub(in crate::legacy) &'a SnapshotOwner,
    pub(in crate::legacy) &'a Grammar,
);
impl crate::writing::managed_writer::ManagedWire for Document<'_> {}
impl Serialize for Document<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let metadata = self.0.metadata();
        let resume = self.0.resume();
        let composer = self.0.composer();
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("version", &1u32)?;
        map.serialize_entry("id", &Read(self.0.id()))?;
        map.serialize_entry("agent_id", &Read(metadata.agent_id()))?;
        optional(&mut map, "title", &metadata.title().map(Read))?;
        optional(&mut map, "name", &metadata.name().map(Read))?;
        if metadata.pinned() {
            map.serialize_entry("pinned", &true)?;
        }
        optional(&mut map, "session_id", &resume.session_id().map(Read))?;
        optional(&mut map, "workdir", &resume.workdir().map(Read))?;
        optional(&mut map, "model", &metadata.model().map(Read))?;
        if !resume.config().is_empty() {
            map.serialize_entry("config", &Choices(resume.config()))?;
        }
        optional(&mut map, "mode", &resume.mode().map(Read))?;
        if resume.pending_history() {
            map.serialize_entry("pending_history", &true)?;
        }
        optional(&mut map, "draft", &composer.draft().map(Read))?;
        map.serialize_entry("updated", &metadata.updated())?;
        map.serialize_entry("entries", &Rows(self.0, self.1))?;
        if !composer.queue().is_empty() {
            map.serialize_entry("queue", &Queue(composer.queue()))?;
        }
        if !composer.attachments().is_empty() {
            map.serialize_entry("attachments", &Attachments(composer.attachments()))?;
        }
        let skip = self.0.rows().len().saturating_sub(MAX_SAVED_ENTRIES);
        let times = self.0.times().get(skip..).unwrap_or_default();
        if !times.is_empty() {
            map.serialize_entry("times", times)?;
        }
        map.end()
    }
}
struct Choices<'a>(&'a [ChoiceSlot]);
impl Serialize for Choices<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for choice in self.0 {
            match choice.value() {
                ChoiceValueSlot::Flag(value) => map.serialize_entry(&Read(choice.key()), value)?,
                ChoiceValueSlot::Value(value) => {
                    map.serialize_entry(&Read(choice.key()), &Read(value.read()))?
                }
            }
        }
        map.end()
    }
}
struct Rows<'a>(&'a SnapshotOwner, &'a Grammar);
impl Serialize for Rows<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let rows = self.0.rows();
        let rows = &rows[rows.len().saturating_sub(MAX_SAVED_ENTRIES)..];
        let mut seq = serializer.serialize_seq(Some(rows.len()))?;
        for row in rows {
            seq.serialize_element(&Row(row, self.1))?;
        }
        seq.end()
    }
}
struct Queue<'a>(&'a [nocterm_ai::history::physical::PromptSlot]);
impl Serialize for Queue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for prompt in self.0 {
            seq.serialize_element(&Prompt(prompt))?;
        }
        seq.end()
    }
}
