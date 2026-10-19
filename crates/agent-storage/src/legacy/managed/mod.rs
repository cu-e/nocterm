//! Historical grammar borrows the genuine managed payloads and original provenance.
mod content;
mod document;
mod tools;
use super::borrowed::Grammar;
pub(super) use document::Document;
use nocterm_ai::{
    history::{CompatibilityReason, physical::*},
    model::{content::MediaKind, extensions::Extensions},
};
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use std::ops::Deref;
struct Read<T>(T);
impl<T: Deref> Serialize for Read<T>
where
    T::Target: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.deref().serialize(serializer)
    }
}
fn optional<M: SerializeMap, T: Serialize>(
    map: &mut M,
    key: &'static str,
    value: &Option<T>,
) -> Result<(), M::Error> {
    if let Some(value) = value {
        map.serialize_entry(key, value)?;
    }
    Ok(())
}
fn metadata<M: SerializeMap>(
    map: &mut M,
    value: Option<PayloadGuard<'_, Extensions>>,
) -> Result<(), M::Error> {
    if let Some(value) = value {
        map.serialize_entry("_meta", &Read(value))?;
    }
    Ok(())
}
pub(super) fn check_row(row: &RowSlot) -> Result<(), CompatibilityReason> {
    match row.view() {
        RowBody::User(values) => values.contents().try_for_each(content::check),
        RowBody::Content(value) => content::check(value),
        RowBody::Tool(value) => tools::check(value),
        RowBody::Agent(_) | RowBody::Thought(_) => Ok(()),
    }
}
pub(super) fn check_prompt(prompt: &PromptSlot) -> Result<(), CompatibilityReason> {
    prompt.images().try_for_each(content::check)
}
pub(super) struct Row<'a>(pub(super) &'a RowSlot, pub(super) &'a Grammar);
impl Serialize for Row<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        match self.0.view() {
            RowBody::Agent(value) => {
                map.serialize_entry("kind", "agent")?;
                map.serialize_entry("value", &Read(value.read()))?;
            }
            RowBody::Thought(value) => {
                map.serialize_entry("kind", "thought")?;
                map.serialize_entry("value", &Read(value.read()))?;
            }
            RowBody::User(values) => {
                map.serialize_entry("kind", "user")?;
                map.serialize_entry("value", &Blocks(values))?;
            }
            RowBody::Content(value) => {
                map.serialize_entry("kind", "content")?;
                map.serialize_entry("value", &Block(value))?;
            }
            RowBody::Tool(value) => {
                map.serialize_entry("kind", "tool")?;
                map.serialize_entry(
                    "value",
                    &tools::Borrowed(value, self.1.managed_order(value)),
                )?;
            }
        }
        map.end()
    }
}
struct Block<'a>(&'a ContentSlot);
impl Serialize for Block<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if matches!(self.0, ContentSlot::Media(value) if value.kind() == MediaKind::Image && value.encoded().len() > 512 * 1024)
        {
            #[derive(Serialize)]
            struct Omitted {
                #[serde(rename = "type")]
                kind: &'static str,
                text: &'static str,
            }
            Omitted {
                kind: "text",
                text: "[Image not kept in history]",
            }
            .serialize(serializer)
        } else {
            content::Borrowed(self.0).serialize(serializer)
        }
    }
}
struct Blocks<'a>(&'a UserSlot);
impl Serialize for Blocks<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0.contents() {
            seq.serialize_element(&Block(value))?;
        }
        seq.end()
    }
}
pub(super) struct Prompt<'a>(pub(super) &'a PromptSlot);
impl Serialize for Prompt<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("id", &self.0.id())?;
        map.serialize_entry("text", &Read(self.0.text()))?;
        map.serialize_entry("images", &Images(self.0))?;
        map.serialize_entry("attachments", &Attachments(self.0.attachments()))?;
        map.end()
    }
}
struct Images<'a>(&'a PromptSlot);
impl Serialize for Images<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.images().len()))?;
        for value in self.0.images() {
            seq.serialize_element(&content::Borrowed(value))?;
        }
        seq.end()
    }
}
struct Attachments<'a>(&'a [AttachmentSlot]);
impl Serialize for Attachments<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&Attachment(value))?;
        }
        seq.end()
    }
}
struct Attachment<'a>(&'a AttachmentSlot);
impl Serialize for Attachment<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let kind = match self.0 {
            AttachmentSlot::Connection(_) => "connection",
            AttachmentSlot::Group(_) => "group",
            AttachmentSlot::LocalTerminal(_) => "local_terminal",
        };
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("kind", kind)?;
        map.serialize_entry("id", &Read(self.0.id()))?;
        map.end()
    }
}
