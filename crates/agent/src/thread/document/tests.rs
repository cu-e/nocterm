//! Semantic transitions, capture offers and publication acknowledgements are distinct.
use super::*;

const ID: &str = "00000000-0000-4000-8000-000000000001";
fn document() -> (RetentionAuthority, ChatDocument) {
    let authority = RetentionAuthority::new(nocterm_ai::history::MAX_RESIDENT_BYTES).unwrap();
    let document = ChatDocument::new(&authority, "agent", ID, 40).unwrap();
    (authority, document)
}

#[test]
fn an_unchanged_edit_does_not_advance_the_desired_revision() {
    let (_, mut document) = document();
    assert!(document.clean());
    assert!(document.set_draft("hello", 41).unwrap());
    let revision = document.desired_revision();
    assert!(!document.set_draft("hello", 42).unwrap());
    assert!(document.rename(Some("name"), 43).unwrap());
    assert!(!document.rename(Some("name"), 44).unwrap());
    assert!(document.pin(true, 45).unwrap());
    assert!(!document.pin(true, 46).unwrap());
    assert_eq!(document.desired_revision(), revision + 2);
}

#[test]
fn a_never_saved_empty_chat_has_nothing_to_persist() {
    let (_, mut document) = document();
    assert_eq!(document.prepare_candidate().unwrap(), None);
    assert!(document.set_draft("text", 41).unwrap());
    assert!(document.set_draft("", 42).unwrap());
    assert_eq!(document.prepare_candidate().unwrap(), None);
}

#[test]
fn repeated_capture_shares_the_revision_until_a_newer_change_arrives() {
    let (_, mut document) = document();
    document.set_draft("first", 41).unwrap();
    let first = document.prepare_candidate().unwrap().unwrap();
    assert_eq!(document.prepare_candidate().unwrap(), Some(first));
    assert_eq!(document.existing_offer(), Some(first));
    assert!(!document.needs_candidate());
    document.set_draft("second", 42).unwrap();
    assert!(document.desired_revision() > first);
    assert!(document.needs_candidate());
}

#[test]
fn an_old_failure_cannot_settle_newer_desired_state() {
    let (_, mut document) = document();
    document.set_draft("first", 41).unwrap();
    let old = document.prepare_candidate().unwrap().unwrap();
    document.set_draft("second", 42).unwrap();
    let error = crate::runtime::AgentPersistenceError::Storage(
        nocterm_ai::storage::AgentStorageError::Interrupted,
    );
    document.failed(old, error);
    assert!(!document.clean());
    assert!(document.desired_revision() > old);
    assert!(document.published_revision() < document.desired_revision());
}

#[test]
fn publication_of_the_desired_revision_leaves_the_document_clean() {
    let (_, mut document) = document();
    document.set_draft("first", 41).unwrap();
    let revision = document.prepare_candidate().unwrap().unwrap();
    assert!(document.published(revision));
    assert_eq!(document.published_revision(), revision);
    assert!(document.clean());
}

fn failure() -> crate::runtime::AgentPersistenceError {
    crate::runtime::AgentPersistenceError::Storage(
        nocterm_ai::storage::AgentStorageError::Interrupted,
    )
}
fn draft(document: &ChatDocument) -> Option<String> {
    document
        .composer()
        .and_then(|composer| composer.draft().map(|draft| draft.to_string()))
}

#[test]
fn repeated_capture_and_failed_retry_share_the_original_revision_and_snapshot() {
    let (_, mut document) = document();
    document.rename(Some("changed"), 81).unwrap();
    let revision = document.prepare_candidate().unwrap().unwrap();
    assert_eq!(document.prepare_candidate().unwrap(), Some(revision));
    assert!(document.mark_normal_offer(revision));
    assert!(!document.mark_normal_offer(revision));
    assert!(
        document.existing_offer().is_some(),
        "scheduling is not admission"
    );
    document.acknowledge_admission(revision);
    assert!(document.failed(revision, failure()));
    assert_eq!(document.prepare_candidate().unwrap(), Some(revision));
    assert!(document.offer(ManagedWriteIntent::Snapshot).is_ok());
    assert_eq!(document.updated(), 81);
}

#[test]
fn old_failure_before_new_admission_and_old_success_cannot_settle_newer_desired_state() {
    let (_, mut document) = document();
    document.rename(Some("first"), 81).unwrap();
    let old = document.prepare_candidate().unwrap().unwrap();
    document.acknowledge_admission(old);
    document.rename(Some("new desired"), 82).unwrap();
    assert!(!document.failed(old, failure()));
    assert!(document.persistence_error().is_none());
    let new = document.prepare_candidate().unwrap().unwrap();
    assert!(new > old);
    assert!(document.failed(new, failure()));
    assert!(!document.published(old));
    assert!(document.persistence_error().is_some());
    assert!(!document.clean());
    assert!(document.published(new));
    assert!(document.persistence_error().is_none());
    assert!(document.clean());
}

#[test]
fn admission_of_an_old_offer_does_not_consume_the_new_literal_draft_change() {
    let (authority, mut document) = document();
    document.rename(Some("old offered name"), 81).unwrap();
    let old = document.prepare_candidate().unwrap().unwrap();
    document.set_draft("new literal draft", 82).unwrap();
    document.acknowledge_admission(old);
    assert!(document.draft_changed());
    assert!(!document.clean());
    assert!(!document.evict_clean(&authority));
    assert!(document.prepare_candidate().unwrap().unwrap() > old);
    assert_eq!(draft(&document).as_deref(), Some("new literal draft"));
}

#[test]
fn disabled_release_retains_unadmitted_or_failed_input_and_an_old_ack_cannot_release_new_input() {
    let (authority, mut document) = document();
    document.set_draft("retained input", 81).unwrap();
    let old = document.prepare_candidate().unwrap().unwrap();
    assert!(!document.release_disabled(&authority));
    assert!(document.failed(old, failure()));
    assert!(!document.release_disabled(&authority));
    assert_eq!(draft(&document).as_deref(), Some("retained input"));
    document.acknowledge_admission(old);
    document.rename(Some("newer desire"), 82).unwrap();
    document.acknowledge_admission(old);
    assert!(!document.release_disabled(&authority));
    let current = document.prepare_candidate().unwrap().unwrap();
    document.acknowledge_admission(current);
    assert!(
        !document.release_disabled(&authority),
        "admission is not publication; a failed write retries from this snapshot"
    );
    assert!(document.failed(current, failure()));
    assert_eq!(document.prepare_candidate().unwrap(), Some(current));
    assert!(document.offer(ManagedWriteIntent::Snapshot).is_ok());
    assert!(document.published(current));
    assert!(document.release_disabled(&authority));
    assert_eq!(draft(&document), None);
    assert_eq!(document.name().as_deref(), Some("newer desire"));
}

#[test]
fn pending_history_stamps_the_chat_only_when_it_changes() {
    let (_, mut document) = document();
    let revision = document.desired_revision();
    assert!(document.set_pending_history(true, 81).unwrap());
    assert!(document.pending_history());
    assert_eq!(document.updated(), 81);
    assert!(document.desired_revision() > revision);
    let revision = document.desired_revision();
    assert!(!document.set_pending_history(true, 82).unwrap());
    assert_eq!(document.updated(), 81);
    assert_eq!(document.desired_revision(), revision);
}

#[test]
fn eviction_refuses_unwritten_input_and_releases_a_published_chat() {
    let (authority, mut document) = document();
    document.rename(Some("kept"), 81).unwrap();
    assert!(!document.evict_clean(&authority));
    let revision = document.prepare_candidate().unwrap().unwrap();
    document.acknowledge_admission(revision);
    assert!(!document.evict_clean(&authority));
    assert!(document.published(revision));
    assert!(document.evict_clean(&authority));
    assert_eq!(document.name().as_deref(), Some("kept"));
    assert_eq!(document.updated(), 81);
}

#[test]
fn the_accepted_prompt_model_is_recorded_once_and_survives_the_snapshot() {
    let (_, mut document) = document();
    let choices = nocterm_ai::session_config::Choices::default();
    let facts = |model| ResumeFacts {
        session: None,
        workdir: None,
        mode: None,
        model,
        choices: &choices,
    };
    assert!(document.sync_resume(facts(Some("opus"))).unwrap());
    let revision = document.desired_revision();
    assert!(!document.sync_resume(facts(Some("opus"))).unwrap());
    assert_eq!(document.desired_revision(), revision);
    let model = document
        .snapshot()
        .and_then(|snapshot| snapshot.metadata().model().map(|model| model.to_string()));
    assert_eq!(model.as_deref(), Some("opus"));
}
