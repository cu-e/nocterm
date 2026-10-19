//! Borrowed pending payloads, never subject to transcript image omission.
use super::content_borrowed as content;
use nocterm_ai::history::{CompatibilityReason, HistoryPrompts, SavedPrompt};
use serde::{
    Serialize,
    ser::{SerializeMap, SerializeSeq},
};
pub(super) fn check(value: &SavedPrompt) -> Result<(), CompatibilityReason> {
    for value in &value.images {
        content::check(value)?;
    }
    Ok(())
}
pub(super) struct Borrowed<'a>(pub(super) &'a SavedPrompt);
impl Serialize for Borrowed<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = self.0;
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("id", &value.id)?;
        map.serialize_entry("text", &value.text)?;
        map.serialize_entry("images", &Images(&value.images))?;
        map.serialize_entry("attachments", &value.attachments)?;
        map.end()
    }
}
struct Images<'a>(&'a [nocterm_ai::model::content::Content]);
impl Serialize for Images<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&content::Borrowed(value))?;
        }
        seq.end()
    }
}
pub(super) struct Queue<'a>(pub(super) &'a dyn HistoryPrompts);
impl Serialize for Queue<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for index in 0..self.0.len() {
            let value = self
                .0
                .get(index)
                .ok_or_else(|| serde::ser::Error::custom("prepared prompt is missing"))?;
            seq.serialize_element(&Borrowed(value))?;
        }
        seq.end()
    }
}
