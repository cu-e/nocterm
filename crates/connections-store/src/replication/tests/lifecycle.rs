use super::*;
use nocterm_connections_model::repository::{
    ConnectionDocument, ConnectionStorageError, StorageOperation,
};

#[gpui::test]
async fn a_restart_continues_from_the_persisted_state_without_resending(
    cx: &mut gpui::TestAppContext,
) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    let saved = fixture.persisted_state();
    let cursor = saved.cursor().cloned().expect("a cursor was kept");
    let added = profile("added");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, added.clone());
    let restarted = Fixture::resumed(
        cx,
        std::slice::from_ref(&original),
        saved,
        fixture.replica.clone(),
    );
    restarted.attach(cx);
    assert_eq!(restarted.local(cx, &added), Some(added));
    assert_eq!(restarted.put_count(), 1);
    assert!(
        restarted
            .replica
            .calls()
            .contains(&ReplicaCall::Changes(Some(cursor.as_str().to_owned())))
    );
}

#[gpui::test]
async fn detaching_stops_synchronizing_and_forgets_the_agreement(cx: &mut gpui::TestAppContext) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture
        .replication
        .update(cx, |replication, cx| replication.detach(cx));
    cx.run_until_parked();
    assert!(matches!(fixture.status(cx), ReplicationStatus::Detached));
    let local = profile("local");
    fixture.save(cx, local.clone()).await;
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, profile("remote"));
    cx.run_until_parked();
    assert_eq!(fixture.put_count(), 1);
    assert_eq!(fixture.local(cx, &profile("remote")), None);
    assert!(fixture.replica.profile(local.id).is_none());
    assert!(fixture.persisted_state().entry(original.id).is_none());
    let scope = fixture
        .replication
        .read_with(cx, |replication, cx| replication.scope_of(original.id, cx));
    assert_eq!(scope, None);
}

#[gpui::test]
async fn detaching_takes_the_profiles_of_that_replica_off_the_device(
    cx: &mut gpui::TestAppContext,
) {
    let agreed = profile("agreed");
    let fixture = Fixture::new(cx, std::slice::from_ref(&agreed));
    fixture.attach(cx);
    fixture
        .replication
        .update(cx, |replication, cx| replication.detach(cx));
    cx.run_until_parked();
    assert_eq!(fixture.local(cx, &agreed), None);
    assert_eq!(fixture.replica.profile(agreed.id), Some(agreed.clone()));
    fixture.attach(cx);
    assert_eq!(fixture.local(cx, &agreed), Some(agreed));
}

#[gpui::test]
async fn attaching_another_replica_moves_synchronization_to_it(cx: &mut gpui::TestAppContext) {
    let original = profile("a");
    let fixture = Fixture::new(cx, std::slice::from_ref(&original));
    fixture.attach(cx);
    fixture
        .replication
        .update(cx, |replication, cx| replication.detach(cx));
    let second = Arc::new(MemoryReplica::new());
    fixture
        .replication
        .update(cx, |replication, cx| replication.attach(second.clone(), cx));
    cx.run_until_parked();
    // What was agreed with the first replica belongs to it, not to the second.
    assert_eq!(second.profile(original.id), None);
    assert_eq!(fixture.local(cx, &original), None);
    let added = profile("added");
    fixture.save(cx, added.clone()).await;
    assert_eq!(second.profile(added.id), Some(added));
    assert_eq!(fixture.put_count(), 1);
}

#[gpui::test]
async fn a_profile_applied_from_an_organization_remembers_its_scope(cx: &mut gpui::TestAppContext) {
    let fixture = Fixture::new(cx, &[]);
    let shared = profile("shared");
    let scope = ReplicaScope::Organization("acme".into());
    fixture.replica.write_remote(scope.clone(), shared.clone());
    fixture.attach(cx);
    let found = fixture
        .replication
        .read_with(cx, |replication, cx| replication.scope_of(shared.id, cx));
    assert_eq!(found, Some(scope.clone()));
    let edited = at_host(&shared, "edited.example");
    fixture.save(cx, edited).await;
    let stored = fixture.replica.stored(shared.id).unwrap();
    assert_eq!(stored.0, scope);
}

#[gpui::test]
async fn changes_are_read_page_by_page_until_none_remain(cx: &mut gpui::TestAppContext) {
    let fixture = Fixture::new(cx, &[]);
    let remote: Vec<Profile> = ["a", "b", "c"].iter().map(|name| profile(name)).collect();
    for profile in &remote {
        fixture
            .replica
            .write_remote(ReplicaScope::Personal, profile.clone());
    }
    fixture.replica.page_size(1);
    fixture.attach(cx);
    for profile in &remote {
        assert_eq!(fixture.local(cx, profile), Some(profile.clone()));
    }
    assert!(matches!(fixture.status(cx), ReplicationStatus::Synced));
}

#[gpui::test]
async fn nothing_is_synchronized_while_the_local_document_failed_to_load(
    cx: &mut gpui::TestAppContext,
) {
    let kept = profile("kept");
    let mut state = ReplicaState::default();
    state.agree(
        kept.id,
        SyncedEntry {
            scope: ReplicaScope::Personal,
            version: ReplicaVersion::new(1),
            profile: kept.clone(),
        },
    );
    state.advance(ReplicaCursor::new("1"));
    let replica = Arc::new(MemoryReplica::new());
    replica.write_remote(ReplicaScope::Personal, kept.clone());
    let unreadable = ConnectionStore::prepared(
        Profiles::default(),
        Recents::default(),
        Arc::new(|_| Box::pin(async { Ok(()) })),
        Arc::new(|_| Box::pin(async { Ok(()) })),
        Some(Arc::new(ConnectionStorageError::new(
            ConnectionDocument::Profiles,
            StorageOperation::Read,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        ))),
        Arc::new(nocterm_connections_model::value_ports::SuppliedValues::new(
            1, 0,
        )),
    );
    let fixture = Fixture::over(cx, unreadable, state, replica);
    fixture.attach(cx);
    assert_eq!(fixture.replica.writes(), []);
    assert!(matches!(
        fixture.status(cx),
        ReplicationStatus::Failed(ReplicationFailure::LocalUnavailable)
    ));
}
