//! Draft text extends the existing wire format without changing old chats.
use super::*;

#[test]
fn legacy_chats_have_no_draft_and_literal_unsent_text_round_trips() {
    let mut chat = SavedChat::new("codex".into());
    chat.entries.push(Entry::Agent("existing answer".into()));
    let old = serde_json::to_value(&chat).unwrap();
    assert!(old.get("draft").is_none());
    let restored: SavedChat = serde_json::from_value(old).unwrap();
    assert!(restored.draft.is_none());
    let draft = " \n\r\t\"\\\0界 ";
    chat.draft = Some(draft.into());
    let dir = tempfile::tempdir().unwrap();
    save(dir.path(), &chat).unwrap();
    let restored = load_all(dir.path());
    assert_eq!(restored[0].draft.as_deref(), Some(draft));
    assert_eq!(restored[0].entries.len(), 1);
}
