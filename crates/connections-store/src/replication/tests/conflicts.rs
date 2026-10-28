use super::*;

fn failed_with(status: ReplicationStatus) -> Option<ReplicationFailure> {
    match status {
        ReplicationStatus::Failed(failure) => Some(failure),
        _ => None,
    }
}

#[gpui::test]
async fn a_write_that_lost_a_race_is_reconciled_against_the_change_it_missed(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    let remote_edit = at_host(&original, "remote.example");
    fixture
        .replica
        .write_concurrently_before_next_write(ReplicaScope::Personal, remote_edit.clone());
    let local_edit = at_host(&original, "local.example");
    fixture.save(cx, local_edit.clone()).await;
    assert_eq!(fixture.local(cx, &original), Some(remote_edit));
    assert_eq!(fixture.put_count(), 2);
    assert!(matches!(
        fixture.events().as_slice(),
        [ReplicationEvent::Overwritten { discarded, .. }] if **discarded == local_edit
    ));
    assert!(matches!(fixture.status(cx), ReplicationStatus::Synced));
}

#[gpui::test]
async fn a_write_that_keeps_conflicting_is_reported_after_three_attempts(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture.replica.conflict_next_writes(10);
    fixture.save(cx, at_host(&original, "local.example")).await;
    assert_eq!(fixture.put_count(), 1 + 3);
    let failure = failed_with(fixture.status(cx));
    assert!(matches!(failure, Some(ReplicationFailure::Conflicts)));
    assert!(matches!(
        fixture.events().as_slice(),
        [ReplicationEvent::Failed(ReplicationFailure::Conflicts)]
    ));
}

#[gpui::test]
async fn an_unreadable_remote_profile_never_changes_the_local_one(cx: &mut gpui::TestAppContext) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture
        .replica
        .write_unrecognized(ReplicaScope::Personal, original.id);
    cx.run_until_parked();
    assert_eq!(fixture.local(cx, &original), Some(original.clone()));
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::NeedsNewerClient
    ));
    let edited = at_host(&original, "local.example");
    fixture.save(cx, edited.clone()).await;
    assert_eq!(fixture.put_count(), 1);
    assert_eq!(fixture.local(cx, &original), Some(edited));
}

#[gpui::test]
async fn an_unreadable_profile_stays_unreadable_across_a_restart_and_clears_when_readable(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture
        .replica
        .write_unrecognized(ReplicaScope::Personal, original.id);
    cx.run_until_parked();
    let restarted = Fixture::resumed(
        cx,
        std::slice::from_ref(&original),
        fixture.persisted_state(),
        fixture.replica.clone(),
    );
    restarted.attach(cx);
    assert!(matches!(
        restarted.status(cx),
        ReplicationStatus::NeedsNewerClient
    ));
    let readable = at_host(&original, "newer.example");
    restarted
        .replica
        .write_remote(ReplicaScope::Personal, readable.clone());
    cx.run_until_parked();
    assert!(matches!(restarted.status(cx), ReplicationStatus::Synced));
    assert_eq!(restarted.local(cx, &original), Some(readable));
}

#[gpui::test]
async fn an_unreachable_replica_leaves_local_work_alone_and_catches_up_when_back(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.replica.set_offline(true);
    fixture.attach(cx);
    assert!(matches!(fixture.status(cx), ReplicationStatus::Offline));
    let added = profile("b");
    fixture.save(cx, added.clone()).await;
    assert_eq!(fixture.local(cx, &added), Some(added.clone()));
    assert_eq!(fixture.replica.writes(), []);
    fixture.replica.set_offline(false);
    cx.run_until_parked();
    assert!(matches!(fixture.status(cx), ReplicationStatus::Synced));
    assert_eq!(fixture.replica.profile(original.id), Some(original));
    assert_eq!(fixture.replica.profile(added.id), Some(added));
    assert_eq!(fixture.events().len(), 0);
}

#[gpui::test]
async fn a_forbidden_profile_is_reported_without_blocking_the_others(
    cx: &mut gpui::TestAppContext,
) {
    let (shared, own) = (profile("shared"), profile("own"));
    let fixture = Fixture::new(cx, &[shared.clone(), own.clone()]);
    fixture.replica.refuse_writes_of(shared.id);
    fixture.attach(cx);
    assert_eq!(fixture.replica.profile(own.id), Some(own));
    assert_eq!(fixture.replica.profile(shared.id), None);
    assert_eq!(fixture.local(cx, &shared), Some(shared.clone()));
    let events = fixture.events();
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| matches!(
        event,
        ReplicationEvent::Failed(ReplicationFailure::Replica(
            nocterm_connections_model::replica::ReplicaError::Forbidden { id }
        )) if *id == shared.id
    )));
}

#[gpui::test]
async fn a_remote_profile_the_local_document_refuses_is_reported_and_skipped(
    cx: &mut gpui::TestAppContext,
) {
    let fixture = Fixture::new(cx, &[]);
    let mut oversized = profile("big");
    oversized.description = "x".repeat(70 * 1024);
    let fine = profile("fine");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, oversized.clone());
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, fine.clone());
    fixture.attach(cx);
    assert_eq!(fixture.local(cx, &oversized), None);
    assert_eq!(fixture.local(cx, &fine), Some(fine));
    assert!(matches!(
        fixture.events().as_slice(),
        [ReplicationEvent::Failed(ReplicationFailure::Rejected { id, .. })] if *id == oversized.id
    ));
}

/// A remote edit of `original` that the local document refuses for its size.
fn oversized_edit(original: &Profile) -> Profile {
    let mut edited = original.clone();
    edited.description = "x".repeat(70 * 1024);
    edited
}

fn rejected_ids(events: &[ReplicationEvent]) -> Vec<ProfileId> {
    events
        .iter()
        .filter_map(|event| match event {
            ReplicationEvent::Failed(ReplicationFailure::Rejected { id, .. }) => Some(*id),
            _ => None,
        })
        .collect()
}

#[gpui::test]
async fn a_remote_change_the_local_document_refuses_keeps_the_cursor_behind_it_and_is_reported_once(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    let before = fixture.persisted_state().cursor().cloned();
    assert!(before.is_some(), "the first pass recorded a position");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, oversized_edit(&original));
    cx.run_until_parked();
    assert_eq!(rejected_ids(&fixture.events()), [original.id]);
    assert_eq!(fixture.persisted_state().cursor().cloned(), before);
    let other = profile("b");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, other.clone());
    cx.run_until_parked();
    assert_eq!(fixture.local(cx, &other), Some(other));
    assert_eq!(fixture.local(cx, &original), Some(original.clone()));
    assert_eq!(
        rejected_ids(&fixture.events()),
        [original.id],
        "the refused change is read again by every pass but reported once"
    );
    assert_eq!(fixture.persisted_state().cursor().cloned(), before);
}

#[gpui::test]
async fn a_local_edit_after_a_refused_remote_change_does_not_end_in_conflicts(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, oversized_edit(&original));
    cx.run_until_parked();
    let puts = fixture.put_count();
    let edited = at_host(&original, "local.example");
    fixture.save(cx, edited.clone()).await;
    assert_eq!(fixture.local(cx, &original), Some(edited));
    assert_eq!(
        fixture.put_count(),
        puts,
        "nothing is sent over a newer version"
    );
    assert!(
        !matches!(
            failed_with(fixture.status(cx)),
            Some(ReplicationFailure::Conflicts)
        ),
        "the edit is not stuck retrying against the missed change"
    );
}
