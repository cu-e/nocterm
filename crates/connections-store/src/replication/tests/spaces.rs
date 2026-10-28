use super::*;
use nocterm_connections_model::replica::{ReplicaAccess, ReplicaSpace};

fn acme() -> ReplicaScope {
    ReplicaScope::Organization("acme".into())
}

fn in_acme(fixture: &Fixture, profile: &Profile) -> Option<Profile> {
    match fixture.replica.stored_in(&acme(), profile.id)?.1 {
        RemoteBody::Present(profile) => Some(*profile),
        RemoteBody::Deleted | RemoteBody::Unrecognized => None,
    }
}

fn in_personal(fixture: &Fixture, profile: &Profile) -> Option<Profile> {
    match fixture
        .replica
        .stored_in(&ReplicaScope::Personal, profile.id)?
        .1
    {
        RemoteBody::Present(profile) => Some(*profile),
        RemoteBody::Deleted | RemoteBody::Unrecognized => None,
    }
}

/// A group profile as a member of the group first sees it: agreed in the group.
async fn with_group_profile(cx: &mut gpui::TestAppContext, shared: &Profile) -> Fixture {
    let fixture = Fixture::new(cx, &[]);
    fixture.replica.write_remote(acme(), shared.clone());
    fixture.attach(cx);
    assert_eq!(fixture.local(cx, shared), Some(shared.clone()));
    fixture
}

#[gpui::test]
async fn a_new_profile_placed_in_a_group_is_created_in_that_group_only(
    cx: &mut gpui::TestAppContext,
) {
    let fixture = Fixture::new(cx, &[]);
    fixture.attach(cx);
    let added = profile("team");
    fixture.place(cx, &added, acme());
    fixture.save(cx, added.clone()).await;
    assert_eq!(in_acme(&fixture, &added), Some(added.clone()));
    assert_eq!(in_personal(&fixture, &added), None);
    assert_eq!(fixture.space_of(cx, &added), acme());
    assert_eq!(
        fixture.persisted_state().entry(added.id).map(|e| &e.scope),
        Some(&acme())
    );
}

#[gpui::test]
async fn a_profile_without_a_requested_scope_stays_in_the_personal_space(
    cx: &mut gpui::TestAppContext,
) {
    let fixture = Fixture::new(cx, &[]);
    fixture.attach(cx);
    let added = profile("mine");
    fixture.save(cx, added.clone()).await;
    assert_eq!(in_personal(&fixture, &added), Some(added.clone()));
    assert_eq!(in_acme(&fixture, &added), None);
    assert_eq!(fixture.space_of(cx, &added), ReplicaScope::Personal);
}

#[gpui::test]
async fn moving_a_synced_profile_into_a_group_deletes_it_from_the_personal_space_first(
    cx: &mut gpui::TestAppContext,
) {
    let mine = profile("mine");
    let fixture = Fixture::new(cx, std::slice::from_ref(&mine));
    fixture.attach(cx);
    let agreed = fixture.replica.stored(mine.id).unwrap().1;
    fixture.place(cx, &mine, acme());
    assert_eq!(
        fixture.replica.writes()[1..],
        [
            ReplicaCall::Delete {
                id: mine.id,
                expected: agreed
            },
            ReplicaCall::Put {
                id: mine.id,
                expected: None
            },
        ]
    );
    assert_eq!(in_personal(&fixture, &mine), None);
    assert_eq!(in_acme(&fixture, &mine), Some(mine.clone()));
    assert_eq!(fixture.local(cx, &mine), Some(mine.clone()));
    assert_eq!(fixture.space_of(cx, &mine), acme());
    assert!(fixture.events().is_empty());
}

#[gpui::test]
async fn moving_a_profile_back_deletes_it_in_the_group_with_the_groups_version(
    cx: &mut gpui::TestAppContext,
) {
    let shared = profile("shared");
    let fixture = with_group_profile(cx, &shared).await;
    let agreed = fixture.replica.stored_in(&acme(), shared.id).unwrap().0;
    fixture.place(cx, &shared, ReplicaScope::Personal);
    assert_eq!(
        fixture.replica.writes(),
        [
            ReplicaCall::Delete {
                id: shared.id,
                expected: agreed
            },
            ReplicaCall::Put {
                id: shared.id,
                expected: None
            },
        ]
    );
    assert_eq!(in_personal(&fixture, &shared), Some(shared.clone()));
    assert_eq!(in_acme(&fixture, &shared), None);
}

#[gpui::test]
async fn asking_for_the_scope_a_profile_already_lives_in_moves_nothing(
    cx: &mut gpui::TestAppContext,
) {
    let shared = profile("shared");
    let fixture = with_group_profile(cx, &shared).await;
    fixture.place(cx, &shared, acme());
    assert_eq!(fixture.replica.writes(), []);
    assert_eq!(fixture.space_of(cx, &shared), acme());
}

#[gpui::test]
async fn a_remote_edit_wins_over_a_pending_move_and_keeps_the_profile_where_it_was(
    cx: &mut gpui::TestAppContext,
) {
    let mine = profile("mine");
    let fixture = Fixture::new(cx, std::slice::from_ref(&mine));
    fixture.attach(cx);
    fixture.replica.set_offline(true);
    fixture.place(cx, &mine, acme());
    let edited = at_host(&mine, "remote.example");
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, edited.clone());
    fixture.replica.set_offline(false);
    cx.run_until_parked();
    assert_eq!(fixture.local(cx, &mine), Some(edited.clone()));
    assert_eq!(in_acme(&fixture, &mine), None);
    assert_eq!(fixture.space_of(cx, &mine), ReplicaScope::Personal);
}

#[gpui::test]
async fn a_refused_edit_in_a_group_returns_to_the_remote_version_and_is_reported_once(
    cx: &mut gpui::TestAppContext,
) {
    let shared = profile("shared");
    let fixture = with_group_profile(cx, &shared).await;
    fixture.replica.refuse_writes_in(acme());
    fixture.save(cx, at_host(&shared, "mine.example")).await;
    assert_eq!(fixture.local(cx, &shared), Some(shared.clone()));
    assert_eq!(in_acme(&fixture, &shared), Some(shared.clone()));
    assert!(matches!(
        fixture.events().as_slice(),
        [ReplicationEvent::Refused { scope, restored: true, .. }] if *scope == acme()
    ));
    let writes = fixture.replica.writes().len();
    fixture
        .replica
        .write_remote(ReplicaScope::Personal, profile("other"));
    cx.run_until_parked();
    assert_eq!(fixture.replica.writes().len(), writes, "no retry follows");
    assert_eq!(fixture.events().len(), 1);
    assert!(matches!(fixture.status(cx), ReplicationStatus::Synced));
}

#[gpui::test]
async fn a_refused_deletion_in_a_group_brings_the_profile_back(cx: &mut gpui::TestAppContext) {
    let shared = profile("shared");
    let fixture = with_group_profile(cx, &shared).await;
    fixture.replica.refuse_writes_in(acme());
    fixture.delete(cx, &shared).await;
    assert_eq!(fixture.local(cx, &shared), Some(shared.clone()));
    assert_eq!(in_acme(&fixture, &shared), Some(shared.clone()));
    assert!(matches!(
        fixture.events().as_slice(),
        [ReplicationEvent::Refused { restored: true, .. }]
    ));
}

#[gpui::test]
async fn a_refused_move_into_a_group_leaves_the_profile_in_the_personal_space(
    cx: &mut gpui::TestAppContext,
) {
    let mine = profile("mine");
    let fixture = Fixture::new(cx, std::slice::from_ref(&mine));
    fixture.attach(cx);
    fixture.replica.refuse_writes_in(acme());
    fixture.place(cx, &mine, acme());
    assert_eq!(fixture.local(cx, &mine), Some(mine.clone()));
    assert_eq!(in_acme(&fixture, &mine), None);
    assert_eq!(in_personal(&fixture, &mine), Some(mine.clone()));
    assert_eq!(fixture.space_of(cx, &mine), ReplicaScope::Personal);
    assert!(matches!(
        fixture.events().as_slice(),
        [ReplicationEvent::Refused { scope, restored: false, .. }] if *scope == acme()
    ));
    assert!(matches!(fixture.status(cx), ReplicationStatus::Synced));
}

#[gpui::test]
async fn a_new_profile_refused_in_a_group_is_kept_in_the_personal_space(
    cx: &mut gpui::TestAppContext,
) {
    let fixture = Fixture::new(cx, &[]);
    fixture.attach(cx);
    fixture.replica.refuse_writes_in(acme());
    let added = profile("team");
    fixture.place(cx, &added, acme());
    fixture.save(cx, added.clone()).await;
    assert_eq!(in_personal(&fixture, &added), Some(added.clone()));
    assert_eq!(in_acme(&fixture, &added), None);
    assert_eq!(fixture.events().len(), 1);
}

#[gpui::test]
async fn the_replication_reports_the_spaces_of_the_attached_replica_only(
    cx: &mut gpui::TestAppContext,
) {
    let fixture = Fixture::new(cx, &[]);
    let spaces = vec![ReplicaSpace {
        scope: acme(),
        name: "Acme".into(),
        access: ReplicaAccess::Read,
    }];
    fixture.replica.set_spaces(spaces.clone());
    let listed = |cx: &gpui::TestAppContext| {
        fixture
            .replication
            .read_with(cx, |replication, _| replication.spaces())
    };
    assert_eq!(listed(cx), []);
    fixture.attach(cx);
    assert_eq!(listed(cx), spaces);
}

#[gpui::test]
async fn a_staged_move_waits_for_the_save_and_a_withdrawn_one_never_happens(
    cx: &mut gpui::TestAppContext,
) {
    let fixture = Fixture::new(cx, &[]);
    fixture.attach(cx);
    let (kept, dropped) = (profile("kept"), profile("dropped"));
    for added in [&kept, &dropped] {
        fixture.replication.update(cx, |replication, cx| {
            assert!(replication.stage(added.id, acme(), cx));
        });
    }
    cx.run_until_parked();
    assert_eq!(fixture.put_count(), 0, "staging alone sends nothing");

    fixture
        .replication
        .update(cx, |replication, cx| replication.unstage(dropped.id, cx));
    fixture.save(cx, kept.clone()).await;
    fixture.save(cx, dropped.clone()).await;
    assert_eq!(in_acme(&fixture, &kept), Some(kept.clone()));
    assert_eq!(in_acme(&fixture, &dropped), None);
    assert_eq!(in_personal(&fixture, &dropped), Some(dropped));
}
