use super::*;
use nocterm_connections_model::replica::{ReplicaAccess, ReplicaSpace};
use nocterm_session::CredentialId;

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
    fixture.stage(cx, &added, acme());
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
async fn a_move_interrupted_between_deleting_and_creating_ends_in_the_target_scope(
    cx: &mut gpui::TestAppContext,
) {
    let mine = profile("mine");
    let fixture = Fixture::new(cx, std::slice::from_ref(&mine));
    fixture.attach(cx);
    fixture.replica.go_offline_after_writes(1);
    fixture.place(cx, &mine, acme());
    assert!(matches!(fixture.status(cx), ReplicationStatus::Offline));
    assert_eq!(in_personal(&fixture, &mine), None);
    assert_eq!(in_acme(&fixture, &mine), None);
    assert_eq!(fixture.space_of(cx, &mine), acme(), "the request survives");

    fixture.replica.set_offline(false);
    cx.run_until_parked();
    assert_eq!(in_acme(&fixture, &mine), Some(mine.clone()));
    assert_eq!(in_personal(&fixture, &mine), None);
    assert_eq!(fixture.space_of(cx, &mine), acme());
    assert!(fixture.persisted_state().placement(mine.id).is_none());
    assert!(fixture.events().is_empty());
}

#[gpui::test]
async fn a_scope_requested_while_a_round_sends_the_profile_is_not_lost(
    cx: &mut gpui::TestAppContext,
) {
    let mine = profile("mine");
    let fixture = Fixture::new(cx, std::slice::from_ref(&mine));
    fixture.attach(cx);
    let release = fixture.replica.hold_next_write();
    let edited = at_host(&mine, "edited.example");
    fixture
        .store
        .update(cx, |store, cx| store.save_profile(edited.clone(), cx))
        .await
        .unwrap();
    cx.run_until_parked();
    fixture.place(cx, &mine, acme());
    assert_eq!(in_acme(&fixture, &mine), None, "the round is still sending");

    release.send(()).unwrap();
    cx.run_until_parked();
    assert_eq!(in_acme(&fixture, &mine), Some(edited));
    assert_eq!(in_personal(&fixture, &mine), None);
    assert_eq!(fixture.space_of(cx, &mine), acme());
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
    fixture.stage(cx, &added, acme());
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

fn with_secret(profile: &Profile, byte: u8) -> (Profile, CredentialId) {
    let credential = CredentialId::from_bytes([byte; 16]);
    let profile = Profile {
        credential: Some(credential),
        ..profile.clone()
    };
    (profile, credential)
}

#[gpui::test]
async fn a_profile_moving_into_a_group_takes_its_secret_along_before_it_leaves(
    cx: &mut gpui::TestAppContext,
) {
    let (mine, secret) = with_secret(&profile("mine"), 7);
    let fixture = Fixture::new(cx, std::slice::from_ref(&mine));
    fixture.replica.keep_secret(ReplicaScope::Personal, secret);
    fixture.attach(cx);
    fixture.place(cx, &mine, acme());
    assert_eq!(fixture.replica.secret_scope(secret), Some(acme()));
    let calls = fixture.replica.writes();
    assert!(matches!(
        calls[1..],
        [
            ReplicaCall::Carry { .. },
            ReplicaCall::Delete { .. },
            ReplicaCall::Put { .. }
        ]
    ));
    assert_eq!(
        calls[1],
        ReplicaCall::Carry {
            id: mine.id,
            from: ReplicaScope::Personal,
            to: acme(),
            credentials: vec![secret],
        }
    );
    assert_eq!(in_acme(&fixture, &mine), Some(mine.clone()));
    assert!(fixture.events().is_empty());
}

#[gpui::test]
async fn a_profile_moving_back_takes_its_secret_out_of_the_group(cx: &mut gpui::TestAppContext) {
    let (shared, secret) = with_secret(&profile("shared"), 8);
    let fixture = Fixture::new(cx, &[]);
    fixture.replica.write_remote(acme(), shared.clone());
    fixture.replica.keep_secret(acme(), secret);
    fixture.attach(cx);
    fixture.place(cx, &shared, ReplicaScope::Personal);
    assert_eq!(
        fixture.replica.secret_scope(secret),
        Some(ReplicaScope::Personal)
    );
    assert_eq!(in_personal(&fixture, &shared), Some(shared.clone()));
}

#[gpui::test]
async fn a_new_group_profile_keeps_no_secret_in_the_personal_vault(cx: &mut gpui::TestAppContext) {
    let (added, secret) = with_secret(&profile("team"), 9);
    let fixture = Fixture::new(cx, &[]);
    fixture.replica.keep_secret(ReplicaScope::Personal, secret);
    fixture.attach(cx);
    fixture.stage(cx, &added, acme());
    fixture.save(cx, added.clone()).await;
    assert_eq!(fixture.replica.secret_scope(secret), Some(acme()));
    assert_eq!(in_acme(&fixture, &added), Some(added.clone()));
}

#[gpui::test]
async fn a_refused_move_leaves_the_secret_and_the_profile_where_they_are(
    cx: &mut gpui::TestAppContext,
) {
    let (mine, secret) = with_secret(&profile("mine"), 10);
    let fixture = Fixture::new(cx, std::slice::from_ref(&mine));
    fixture.replica.keep_secret(ReplicaScope::Personal, secret);
    fixture.attach(cx);
    fixture.replica.refuse_writes_in(acme());
    fixture.place(cx, &mine, acme());
    assert_eq!(
        fixture.replica.secret_scope(secret),
        Some(ReplicaScope::Personal)
    );
    assert_eq!(in_personal(&fixture, &mine), Some(mine.clone()));
    assert_eq!(in_acme(&fixture, &mine), None);
    assert_eq!(fixture.space_of(cx, &mine), ReplicaScope::Personal);
    assert!(matches!(
        fixture.events().as_slice(),
        [ReplicationEvent::Refused { scope, .. }] if *scope == acme()
    ));
    assert!(matches!(fixture.status(cx), ReplicationStatus::Synced));
}

#[gpui::test]
async fn a_secret_kept_in_another_group_is_not_pulled_into_a_profiles_group(
    cx: &mut gpui::TestAppContext,
) {
    let (added, secret) = with_secret(&profile("team"), 11);
    let other = ReplicaScope::Organization("other".into());
    let fixture = Fixture::new(cx, &[]);
    fixture.replica.keep_secret(other.clone(), secret);
    fixture.attach(cx);
    fixture.stage(cx, &added, acme());
    fixture.save(cx, added.clone()).await;
    assert_eq!(fixture.replica.secret_scope(secret), Some(other));
}

fn scopes(items: &[ReplicaScope]) -> std::collections::HashSet<ReplicaScope> {
    items.iter().cloned().collect()
}

/// Two personal profiles that sign in with one secret.
fn sharing(secret: CredentialId) -> (Profile, Profile) {
    let using = |name: &str| Profile {
        credential: Some(secret),
        ..profile(name)
    };
    (using("first"), using("second"))
}

#[gpui::test]
async fn a_secret_another_profile_still_uses_is_copied_when_a_profile_moves_to_a_group(
    cx: &mut gpui::TestAppContext,
) {
    let secret = CredentialId::from_bytes([12; 16]);
    let (first, second) = sharing(secret);
    let fixture = Fixture::new(cx, &[first.clone(), second.clone()]);
    fixture.replica.keep_secret(ReplicaScope::Personal, secret);
    fixture.attach(cx);
    fixture.place(cx, &first, acme());
    assert_eq!(
        fixture.replica.secret_scopes(secret),
        scopes(&[ReplicaScope::Personal, acme()]),
        "the group has its copy and the profile that stayed keeps its own"
    );
    assert!(fixture.replica.writes().iter().any(|call| matches!(
        call,
        ReplicaCall::Copy { id, credentials, .. } if *id == first.id && *credentials == [secret]
    )));
    assert!(
        !fixture
            .replica
            .writes()
            .iter()
            .any(|call| matches!(call, ReplicaCall::Carry { .. })),
        "nothing was taken from the personal vault"
    );
    assert_eq!(in_acme(&fixture, &first), Some(first));
    assert_eq!(in_personal(&fixture, &second), Some(second));
}

#[gpui::test]
async fn the_last_profile_to_leave_takes_a_shared_secret_out_of_the_source(
    cx: &mut gpui::TestAppContext,
) {
    let secret = CredentialId::from_bytes([13; 16]);
    let (first, second) = sharing(secret);
    let fixture = Fixture::new(cx, &[first.clone(), second.clone()]);
    fixture.replica.keep_secret(ReplicaScope::Personal, secret);
    fixture.attach(cx);
    fixture.place(cx, &first, acme());
    fixture.place(cx, &second, acme());
    assert_eq!(fixture.replica.secret_scope(secret), Some(acme()));
}

#[gpui::test]
async fn a_new_group_profile_copies_a_secret_a_personal_profile_uses(
    cx: &mut gpui::TestAppContext,
) {
    let secret = CredentialId::from_bytes([14; 16]);
    let (mine, team) = sharing(secret);
    let fixture = Fixture::new(cx, std::slice::from_ref(&mine));
    fixture.replica.keep_secret(ReplicaScope::Personal, secret);
    fixture.attach(cx);
    fixture.stage(cx, &team, acme());
    fixture.save(cx, team.clone()).await;
    assert_eq!(
        fixture.replica.secret_scopes(secret),
        scopes(&[ReplicaScope::Personal, acme()])
    );
    assert_eq!(in_acme(&fixture, &team), Some(team));
}

#[gpui::test]
async fn a_refused_copy_leaves_the_shared_secret_and_the_profile_where_they_are(
    cx: &mut gpui::TestAppContext,
) {
    let secret = CredentialId::from_bytes([15; 16]);
    let (first, second) = sharing(secret);
    let fixture = Fixture::new(cx, &[first.clone(), second]);
    fixture.replica.keep_secret(ReplicaScope::Personal, secret);
    fixture.attach(cx);
    fixture.replica.refuse_writes_in(acme());
    fixture.place(cx, &first, acme());
    assert_eq!(
        fixture.replica.secret_scope(secret),
        Some(ReplicaScope::Personal)
    );
    assert_eq!(in_personal(&fixture, &first), Some(first));
}
