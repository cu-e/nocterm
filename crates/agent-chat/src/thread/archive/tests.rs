//! Hydration transitions reject old delivery without consuming newer intentions.
use super::*;
use futures::FutureExt as _;
use nocterm_ai::{history::HistoryGate, session::SessionState};

fn saved() -> SavedChat {
    SavedChat::for_identity(
        "codex".into(),
        "00000000-0000-4000-8000-000000000067".into(),
        1,
    )
}
fn permit(gate: &HistoryGate) -> HistoryPermit {
    gate.acquire()
        .now_or_never()
        .expect("available history delivery")
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
    let gate = HistoryGate::default();
    let old = begin(
        &mut archive,
        ArchiveIntent::Rename {
            name: Some("failed name".into()),
        },
    );
    assert!(matches!(
        archive.accept(
            old,
            true,
            Err(AgentStorageError::Interrupted),
            permit(&gate)
        ),
        LoadOutcome::Failed(_)
    ));
    let retry = begin(&mut archive, ArchiveIntent::Pin { pinned: true });
    assert_ne!(old, retry);
    assert!(matches!(
        archive.accept(old, true, Ok(chat.clone()), permit(&gate)),
        LoadOutcome::Stale
    ));
    let LoadOutcome::Hydrate { intents, .. } = archive.accept(retry, true, Ok(chat), permit(&gate))
    else {
        panic!("retry accepts");
    };
    assert!(matches!(
        intents.as_slice(),
        [ArchiveIntent::Pin { pinned: true }]
    ));
}
#[test]
fn stale_delivery_does_not_take_latest_empty_draft_or_its_timestamp() {
    let chat = saved();
    let mut archive = Archive::archived(ChatSummary::from_chat(&chat));
    let gate = HistoryGate::default();
    let load = begin(&mut archive, ArchiveIntent::Pin { pinned: true });
    archive.edit_draft("earlier draft".into(), 20);
    archive.edit_draft(String::new(), 21);
    assert!(matches!(
        archive.accept(load, false, Ok(chat.clone()), permit(&gate)),
        LoadOutcome::Stale
    ));
    let patch = archive.draft_patch().unwrap();
    assert_eq!(patch.header.updated, 21);
    assert!(
        patch.draft.is_none(),
        "the empty overlay is a real clearing edit"
    );
    let LoadOutcome::Hydrate {
        overlay: Some(overlay),
        intents,
        ..
    } = archive.accept(load, true, Ok(chat), permit(&gate))
    else {
        panic!("current delivery hydrates");
    };
    assert_eq!(overlay.text(), "");
    assert_eq!(overlay.updated(), 21);
    assert_eq!(intents.len(), 1);
}
#[test]
fn accepted_delivery_holds_shared_permit_through_hydration_until_release() {
    let chat = saved();
    let bytes = chat.resident_bytes();
    let mut archive = Archive::archived(ChatSummary::from_chat(&chat));
    let gate = HistoryGate::default();
    let load = begin(&mut archive, ArchiveIntent::Pin { pinned: true });
    assert!(matches!(
        archive.accept(load, true, Ok(chat), permit(&gate)),
        LoadOutcome::Hydrate { .. }
    ));
    assert!(gate.acquire().now_or_never().is_none());
    archive.hydration_committed(bytes);
    assert!(!archive.is_archived());
    assert_eq!(archive.resident_bytes(), bytes);
    assert!(gate.acquire().now_or_never().is_none());
    archive.release_delivery_permit();
    assert!(gate.acquire().now_or_never().is_some());
}
#[test]
fn intentions_wait_for_one_generation_and_keep_accepted_order() {
    let chat = saved();
    let mut archive = Archive::archived(ChatSummary::from_chat(&chat));
    let load = begin(
        &mut archive,
        ArchiveIntent::Rename {
            name: Some("renamed".into()),
        },
    );
    assert!(matches!(
        archive.request(
            ArchiveIntent::Pin { pinned: true },
            SessionState::default().ticket()
        ),
        RequestOutcome::AlreadyLoading
    ));
    let LoadOutcome::Hydrate { intents, .. } =
        archive.accept(load, true, Ok(chat), permit(&HistoryGate::default()))
    else {
        panic!("hydration");
    };
    assert!(
        matches!(intents.as_slice(), [ArchiveIntent::Rename { name: Some(name) }, ArchiveIntent::Pin { pinned: true }] if name == "renamed")
    );
}
#[test]
fn archived_without_edits_has_no_patch_and_failed_load_retains_literal_overlay() {
    let chat = saved();
    let mut archive = Archive::archived(ChatSummary::from_chat(&chat));
    assert!(archive.draft_patch().is_none());
    let load = begin(&mut archive, ArchiveIntent::Pin { pinned: true });
    archive.edit_draft("literal".into(), 30);
    assert!(matches!(
        archive.accept(
            load,
            true,
            Err(AgentStorageError::Interrupted),
            permit(&HistoryGate::default())
        ),
        LoadOutcome::Failed(_)
    ));
    let patch = archive.draft_patch().unwrap();
    assert_eq!(patch.draft.as_deref(), Some("literal"));
    assert_eq!(patch.header.updated, 30);
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
