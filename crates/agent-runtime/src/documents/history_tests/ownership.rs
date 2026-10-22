//! Reservations, duplicate windows and explicit replacement use immutable scoped claims.
use super::contexts::{project_chat, scoped_fixture};
use super::*;
use nocterm_ai::{history::ContextId, storage::AgentStorageError};

pub(super) fn register(
    runtime: &mut Runtime,
    client: Client,
    cx: &mut Context<Runtime>,
) -> Result<(), crate::ChatOpenError> {
    ChatStore::require_client_scope(ContextId::General, &client, cx)?;
    let key = client
        .state(cx)
        .ok_or(crate::ChatOpenError::Closed)?
        .origin()
        .key()
        .clone();
    if runtime.scoped_document_is_current(&key, client.id()) {
        return Ok(());
    }
    let window = client.id(); // The public writer fixture's marker stands in for its root window.
    let claim = owned_claim(runtime.claim_document(key, window, None)?);
    runtime.commit_document(claim, client, cx)
}
pub(super) fn register_bound(
    handle: &RuntimeHandle,
    client: Client,
    cx: &mut App,
) -> Result<(), crate::ChatOpenError> {
    let state = client.state(cx).ok_or(crate::ChatOpenError::Closed)?;
    let claim =
        owned_claim(handle.claim_document(state.origin().key().id().into(), client.id(), cx)?);
    handle.commit_document(claim, client, cx)
}
pub(super) fn replace(
    runtime: &Entity<Runtime>,
    old: EntityId,
    replacement: Client,
    cx: &mut TestAppContext,
) {
    runtime.update(cx, |runtime, cx| {
        let key = replacement.state(cx).unwrap().origin().key().clone();
        let claim = owned_claim(runtime.claim_document(key, old, Some(old)).unwrap());
        runtime.commit_document(claim, replacement, cx).unwrap();
    });
}

pub(super) fn owned_claim(outcome: crate::DocumentClaimOutcome) -> crate::DocumentClaim {
    match outcome {
        crate::DocumentClaimOutcome::Owned(claim) => claim,
        _ => panic!("expected a new document claim"),
    }
}
#[gpui::test]
fn document_reservations_prevent_duplicate_opening_and_drop_rolls_back_only_the_original_key(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let first_window = cx.new(|_| ());
    let second_window = cx.new(|_| ());
    let id = snapshot().header.id.clone();
    cx.update(|cx| {
        let original = owned_claim(general.claim_document(id.clone(), first_window.entity_id(), cx).unwrap());
        assert!(matches!(general.claim_document(id.clone(), second_window.entity_id(), cx).unwrap(), crate::DocumentClaimOutcome::AlreadyOpen { window } if window == first_window.entity_id()));
        let other = owned_claim(project.claim_document(id.clone(), second_window.entity_id(), cx).unwrap());
        assert_eq!(other.key().scope(), project.scope());
        drop(original);
        let replacement = owned_claim(general.claim_document(id.clone(), second_window.entity_id(), cx).unwrap());
        assert!(matches!(project.claim_document(id, first_window.entity_id(), cx).unwrap(), crate::DocumentClaimOutcome::AlreadyOpen { window } if window == second_window.entity_id()));
        drop(replacement);
        drop(other);
    });
}
#[gpui::test]
fn committed_claims_keep_the_original_window_and_refuse_wrong_registry_or_scope(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    let (another_runtime, _other_directory, _, _, _) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let window = cx.new(|_| ());
    let client = ProbeClient::new(cx, snapshot());
    let id = snapshot().header.id.clone();
    cx.update(|cx| {
        let claim = owned_claim(general.claim_document(id.clone(), window.entity_id(), cx).unwrap());
        assert!(matches!(another_runtime.commit_document(claim, client.clone(), cx), Err(crate::ChatOpenError::StaleClaim(_))));
        let claim = owned_claim(general.claim_document(id.clone(), window.entity_id(), cx).unwrap());
        assert!(matches!(project.commit_document(claim, client.clone(), cx), Err(crate::ChatOpenError::Storage(AgentStorageError::ScopeMismatch { .. }))));
        let claim = owned_claim(general.claim_document(id.clone(), window.entity_id(), cx).unwrap());
        general.commit_document(claim, client.clone(), cx).unwrap();
        assert!(general.document_is_current(&id, client.id(), cx));
        assert!(matches!(general.claim_document(id, client.id(), cx).unwrap(), crate::DocumentClaimOutcome::AlreadyOpen { window: original } if original == window.entity_id()));
    });
}
#[gpui::test]
fn restart_requires_the_exact_live_owner_and_cancelled_replacement_restores_it(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    let window = cx.new(|_| ());
    let old = ProbeClient::new(cx, snapshot());
    let replacement = ProbeClient::new(cx, snapshot());
    let id = snapshot().header.id.clone();
    cx.update(|cx| {
        let claim = owned_claim(
            general
                .claim_document(id.clone(), window.entity_id(), cx)
                .unwrap(),
        );
        general.commit_document(claim, old.clone(), cx).unwrap();
        assert!(matches!(
            general.replace_document(id.clone(), replacement.id(), window.entity_id(), cx),
            Err(crate::ChatOpenError::StaleClaim(_))
        ));
        let aborted = owned_claim(
            general
                .replace_document(id.clone(), old.id(), window.entity_id(), cx)
                .unwrap(),
        );
        assert!(!general.document_is_current(&id, old.id(), cx));
        assert!(matches!(
            general.replace_document(id.clone(), old.id(), window.entity_id(), cx),
            Err(crate::ChatOpenError::StaleClaim(_))
        ));
        drop(aborted);
        assert!(general.document_is_current(&id, old.id(), cx));
        let admitted = owned_claim(
            general
                .replace_document(id.clone(), old.id(), window.entity_id(), cx)
                .unwrap(),
        );
        general
            .commit_document(admitted, replacement.clone(), cx)
            .unwrap();
        general.unregister_document(old.id(), cx).unwrap();
        assert!(general.document_is_current(&id, replacement.id(), cx));
        assert!(matches!(
            general.replace_document(id, old.id(), window.entity_id(), cx),
            Err(crate::ChatOpenError::StaleClaim(_))
        ));
    });
}
#[gpui::test]
fn deletion_seals_a_pending_claim_without_sealing_a_same_uuid_in_another_scope(
    cx: &mut TestAppContext,
) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    let project = general.for_scope(project_chat().header.scope);
    let window = cx.new(|_| ());
    let client = ProbeClient::new(cx, snapshot());
    let id = snapshot().header.id.clone();
    cx.update(|cx| {
        let pending = owned_claim(
            general
                .claim_document(id.clone(), window.entity_id(), cx)
                .unwrap(),
        );
        general.delete_chat(id.clone(), cx);
        assert!(matches!(
            general.commit_document(pending, client.clone(), cx),
            Err(crate::ChatOpenError::Deleted(_))
        ));
        assert!(matches!(
            general
                .claim_document(id.clone(), window.entity_id(), cx)
                .unwrap(),
            crate::DocumentClaimOutcome::Deleted
        ));
        drop(owned_claim(
            project.claim_document(id, window.entity_id(), cx).unwrap(),
        ));
    });
}
#[gpui::test]
fn a_closed_owner_can_be_reopened_before_its_late_retirement_arrives(cx: &mut TestAppContext) {
    let (general, _directory, _, _, _) = scoped_fixture(cx);
    let window = cx.new(|_| ());
    let old = ProbeClient::new(cx, snapshot());
    let next = ProbeClient::new(cx, snapshot());
    let id = snapshot().header.id.clone();
    cx.update(|cx| {
        let claim = owned_claim(
            general
                .claim_document(id.clone(), window.entity_id(), cx)
                .unwrap(),
        );
        general.commit_document(claim, old.clone(), cx).unwrap();
        old.close();
        let claim = owned_claim(
            general
                .claim_document(id.clone(), window.entity_id(), cx)
                .unwrap(),
        );
        general.commit_document(claim, next.clone(), cx).unwrap();
        general.unregister_document(old.id(), cx).unwrap();
        assert!(general.document_is_current(&id, next.id(), cx));
    });
}
#[gpui::test]
async fn shutdown_captures_the_original_document_while_a_replacement_is_reserved(
    cx: &mut TestAppContext,
) {
    let (general, _directory, repository, _, _) = scoped_fixture(cx);
    let window = cx.new(|_| ());
    let old = ProbeClient::new(cx, snapshot());
    let id = snapshot().header.id.clone();
    let (replacement, final_work) = cx.update(|cx| {
        let claim = owned_claim(
            general
                .claim_document(id.clone(), window.entity_id(), cx)
                .unwrap(),
        );
        general.commit_document(claim, old.clone(), cx).unwrap();
        let replacement = owned_claim(
            general
                .replace_document(id.clone(), old.id(), window.entity_id(), cx)
                .unwrap(),
        );
        (replacement, general.capture_shutdown(cx))
    });
    final_work.await.unwrap();
    assert_eq!(repository.transcript(&id).unwrap().draft, snapshot().draft);
    drop(replacement);
}
