use super::*;
use crate::{acp, history::SavedPrompt, thread::Entry};
fn chat() -> SavedChat {
    let mut chat = SavedChat::new("codex".into());
    chat.entries.push(Entry::User(vec![acp::ContentBlock::Text(
        acp::TextContent::new("first message"),
    )]));
    chat
}
#[test]
fn cached_structural_estimates_cover_preflight_and_pending_only_rows() {
    let mut chat = chat();
    chat.queue.push(SavedPrompt {
        id: 1,
        text: "界\n\"".into(),
        images: vec![],
        attachments: vec![],
    });
    let wire = super::super::wire::Wire::new(&chat, Some(&chat.queue));
    let bytes = serde_json::to_vec(&wire).unwrap();
    let preflight = super::super::preflight::check(&bytes).unwrap();
    let estimated = super::super::size::structural_size(&wire).unwrap();
    assert!(estimated >= preflight, "{estimated} < {preflight}");
    assert!(estimated >= bytes.len());
}
