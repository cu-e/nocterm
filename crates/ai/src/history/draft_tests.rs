//! Draft text extends the existing wire format without changing old chats.
use super::*;

#[test]
fn legacy_chats_have_no_draft_and_literal_unsent_text_round_trips() {
    let mut chat = SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-4000-8000-000000000065".into(),
        1,
    );
    chat.entries.push(Entry::Agent("existing answer".into()));
    let old = serde_json::to_value(&chat).unwrap();
    assert!(old.get("draft").is_none());
    let restored: SavedChat = serde_json::from_value(old).unwrap();
    assert!(restored.draft.is_none());
    let draft = " \n\r\t\"\\\0界 ";
    chat.draft = Some(draft.into());
    let encoded = encode(&chat).unwrap();
    let restored = decode(&encoded, &chat.id).unwrap();
    assert_eq!(restored.draft.as_deref(), Some(draft));
    assert_eq!(restored.entries.len(), 1);
}
