//! Hydration transitions reject old delivery without consuming newer intentions.
use super::*;
use crate::thread::document::DocumentError;
use load_operation::ArchiveLoadError;
use nocterm_ai::{history::SavedChat, session::SessionState};

fn refusal() -> ArchiveLoadError {
    ArchiveLoadError::Document(DocumentError::Retired)
}

fn saved() -> SavedChat {
    SavedChat::for_identity(
        &nocterm_ai::model::retention::RetentionAuthority::new(
            nocterm_ai::history::MAX_RESIDENT_BYTES,
        )
        .unwrap(),
        "codex".into(),
        "00000000-0000-4000-8000-000000000067".into(),
        1,
    )
}
fn begin(archive: &mut Archive, intent: ArchiveIntent) -> LoadId {
    match archive.request(intent, SessionState::default().ticket()) {
        RequestOutcome::Start { load_id, .. } => load_id,
        _ => panic!("archived document starts one load"),
    }
}
#[test]
fn failed_generation_can_retry_without_replaying_failed_intentions() {
    let chat = saved();
    let mut archive = Archive::archived(ChatSummary::from_chat(&chat));
    let old = begin(
        &mut archive,
        ArchiveIntent::Rename {
            name: Some("failed name".into()),
        },
    );
    assert!(matches!(
        archive.accept(old, true, Err(refusal()), None),
        LoadOutcome::Refused(_)
    ));
    let retry = begin(&mut archive, ArchiveIntent::Pin { pinned: true });
    assert_ne!(old, retry);
    assert!(matches!(
        archive.accept(old, true, Err(refusal()), None),
        LoadOutcome::Stale
    ));
}
#[test]
fn stale_delivery_does_not_take_latest_empty_draft_or_its_timestamp() {
    let chat = saved();
    let mut archive = Archive::archived(ChatSummary::from_chat(&chat));
    let load = begin(&mut archive, ArchiveIntent::Pin { pinned: true });
    archive.edit_draft("earlier draft".into(), 20);
    archive.edit_draft(String::new(), 21);
    assert!(matches!(
        archive.accept(load, false, Err(refusal()), None),
        LoadOutcome::Stale
    ));
    let patch = archive
        .draft_patch(chat.id(), &chat.metadata().agent_id)
        .unwrap();
    assert_eq!(patch.updated, 21);
    assert!(
        patch.text.is_none(),
        "the empty overlay is a real clearing edit"
    );
}
#[test]
fn archived_without_edits_has_no_patch_and_failed_load_retains_literal_overlay() {
    let chat = saved();
    let mut archive = Archive::archived(ChatSummary::from_chat(&chat));
    assert!(
        archive
            .draft_patch(chat.id(), &chat.metadata().agent_id)
            .is_none()
    );
    let load = begin(&mut archive, ArchiveIntent::Pin { pinned: true });
    archive.edit_draft("literal".into(), 30);
    assert!(matches!(
        archive.accept(load, true, Err(refusal()), None),
        LoadOutcome::Refused(_)
    ));
    let patch = archive
        .draft_patch(chat.id(), &chat.metadata().agent_id)
        .unwrap();
    assert_eq!(patch.text.as_deref(), Some("literal"));
    assert_eq!(patch.updated, 30);
}
#[test]
fn eviction_requires_every_durable_residency_gate() {
    let chat = saved();
    let mut archive = Archive::default();
    for denied in 0..6 {
        let mut allowed = [true; 6];
        allowed[denied] = false;
        let [detached, idle, durable, clean_draft, no_error, saved] = allowed;
        assert!(!archive.request_evict(
            EvictionGate::new(detached, idle, durable, clean_draft, no_error, saved,),
            ChatSummary::from_chat(&chat)
        ));
        assert!(!archive.is_archived());
    }
    assert!(archive.request_evict(
        EvictionGate::new(true, true, true, true, true, true),
        ChatSummary::from_chat(&chat)
    ));
    assert!(archive.is_archived());
}
