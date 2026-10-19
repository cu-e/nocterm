//! Missing draft remains absent; literal unsent text round-trips through the codec.
use super::*;
use crate::thread::Entry;
#[test]
fn missing_draft_is_absent_and_literal_unsent_text_round_trips() {
    let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
    let chat = SavedChat::for_identity(
        &authority,
        "codex".into(),
        "00000000-0000-4000-8000-000000000065".into(),
        1,
    );
    let (id, metadata, resume, mut transcript, composer, _) = chat.into_parts();
    transcript
        .entries
        .push(Entry::Agent("existing answer".into()));
    let chat =
        SavedChat::from_parts(&authority, id, metadata, resume, transcript, composer).unwrap();
    let mut old = serde_json::to_value(&chat).unwrap();
    old["composer"].as_object_mut().unwrap().remove("draft");
    let restored: SavedChat = crate::model::decode::Decode::decode(&authority, old).unwrap();
    assert!(restored.composer().draft.is_none());
    let draft = " \n\r\t\"\\\0界 ";
    let (id, metadata, resume, transcript, mut composer, _) = restored.into_parts();
    composer.draft = Some(draft.into());
    let chat =
        SavedChat::from_parts(&authority, id, metadata, resume, transcript, composer).unwrap();
    let encoded = encode(&chat).unwrap();
    let restored = decode(&authority, &encoded, chat.id()).unwrap();
    assert_eq!(restored.composer().draft.as_deref(), Some(draft));
    assert_eq!(restored.transcript().entries.len(), 1);
}
