//! Borrowed bounded transcript grammar; pending prompts retain full images.
use super::{content_borrowed as content, tools_borrowed as tools};
use nocterm_ai::{
    history::CompatibilityReason,
    model::content::{Content, Media, MediaKind},
    thread::Entry,
};
use serde::{Serialize, ser::SerializeSeq};

pub(super) fn check(value: &Entry) -> Result<(), CompatibilityReason> {
    match value {
        Entry::User(values) => {
            for value in values {
                content::check(value)?;
            }
            Ok(())
        }
        Entry::Content(value) => content::check(value),
        Entry::Tool(value) => tools::check(value),
        Entry::Agent(_) | Entry::Thought(_) => Ok(()),
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(super) enum Borrowed<'a> {
    User(Blocks<'a>),
    Agent(&'a str),
    Thought(&'a str),
    Content(Block<'a>),
    Tool(tools::Borrowed<'a>),
}
pub(super) fn borrowed<'a>(
    value: &'a Entry,
    grammar: &'a super::borrowed::Grammar,
) -> Borrowed<'a> {
    match value {
        Entry::User(value) => Borrowed::User(Blocks(value)),
        Entry::Agent(value) => Borrowed::Agent(value),
        Entry::Thought(value) => Borrowed::Thought(value),
        Entry::Content(value) => Borrowed::Content(Block(value)),
        Entry::Tool(value) => Borrowed::Tool(tools::Borrowed(value, grammar.order(value))),
    }
}
pub(super) struct Rows<'a>(
    pub(super) &'a [Entry],
    pub(super) &'a super::borrowed::Grammar,
);
impl Serialize for Rows<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&borrowed(value, self.1))?;
        }
        seq.end()
    }
}
pub(super) struct Blocks<'a>(&'a [Content]);
impl Serialize for Blocks<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            seq.serialize_element(&Block(value))?;
        }
        seq.end()
    }
}
pub(super) struct Block<'a>(&'a Content);
impl Serialize for Block<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if matches!(self.0, Content::Media(Media { kind: MediaKind::Image, encoded, .. }) if encoded.len() > 512 * 1024)
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
