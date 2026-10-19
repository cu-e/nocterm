//! Transcript variants only move supplied concrete neutral payloads.
use crate::{
    model::{content::Content, native::NativeOwned, tools::ToolRecord},
    thread::Entry,
};
pub fn agent(owned: NativeOwned<String>) -> NativeOwned<Entry> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Entry::Agent(value), allocation)
}
pub fn thought(owned: NativeOwned<String>) -> NativeOwned<Entry> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Entry::Thought(value), allocation)
}
pub fn user(owned: NativeOwned<Vec<Content>>) -> NativeOwned<Entry> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Entry::User(value), allocation)
}
pub fn content_entry(owned: NativeOwned<Content>) -> NativeOwned<Entry> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Entry::Content(value), allocation)
}
pub fn tool_entry(owned: NativeOwned<ToolRecord>) -> NativeOwned<Entry> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(Entry::Tool(value), allocation)
}
