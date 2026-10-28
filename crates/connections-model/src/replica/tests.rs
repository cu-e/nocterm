use super::rules::{ReplicaAction, plan, reconcile, reconcile_placed};
use super::*;
use crate::{profiles::Profiles, test_support::profile_id_for};
use nocterm_session::{Auth, CredentialId, Target};

fn profile(name: &str) -> Profile {
    Profile {
        id: profile_id_for("shared"),
        name: name.into(),
        description: String::new(),
        options: Default::default(),
        target: Target::new("ci", "host", 22),
        auth: Auth::Password,
        credential: None,
        launch: None,
        icon: None,
        icon_color: None,
        country: None,
        group: None,
        jump_hosts: Vec::new(),
    }
}

fn id() -> ProfileId {
    profile_id_for("shared")
}

fn version(version: u64) -> ReplicaVersion {
    ReplicaVersion::new(version)
}

fn synced(name: &str, at: u64) -> SyncedEntry {
    SyncedEntry {
        scope: ReplicaScope::Personal,
        version: version(at),
        profile: profile(name),
    }
}

fn remote(body: RemoteBody, at: u64) -> RemoteProfile {
    RemoteProfile {
        scope: ReplicaScope::Personal,
        id: id(),
        version: version(at),
        body,
    }
}

fn present(name: &str, at: u64) -> RemoteProfile {
    remote(RemoteBody::Present(Box::new(profile(name))), at)
}

fn save(name: &str, at: u64, expected: Option<&str>) -> ReplicaAction {
    ReplicaAction::SaveLocal {
        scope: ReplicaScope::Personal,
        version: version(at),
        profile: Box::new(profile(name)),
        expected: expected.map(|name| Box::new(profile(name))),
    }
}

fn push(name: &str, expected: Option<u64>) -> ReplicaAction {
    ReplicaAction::Push {
        scope: ReplicaScope::Personal,
        profile: Box::new(profile(name)),
        expected: expected.map(version),
    }
}

fn marked(name: &str, at: u64) -> ReplicaAction {
    ReplicaAction::MarkSynced {
        scope: ReplicaScope::Personal,
        version: version(at),
        profile: Box::new(profile(name)),
    }
}

#[test]
fn nothing_happens_when_local_and_remote_equal_the_synced_profile() {
    let local = profile("a");
    let actions = reconcile(Some(&local), Some(&present("a", 3)), Some(&synced("a", 3)));
    assert_eq!(actions, []);
    assert_eq!(reconcile(Some(&local), None, Some(&synced("a", 3))), []);
}

#[test]
fn a_newer_version_with_the_synced_content_only_updates_the_agreed_version() {
    let local = profile("a");
    let actions = reconcile(Some(&local), Some(&present("a", 4)), Some(&synced("a", 3)));
    assert_eq!(actions, [marked("a", 4)]);
}

#[test]
fn a_remote_change_is_applied_over_an_unchanged_local_profile() {
    let local = profile("a");
    let actions = reconcile(Some(&local), Some(&present("b", 4)), Some(&synced("a", 3)));
    assert_eq!(actions, [save("b", 4, Some("a"))]);
}

#[test]
fn a_local_change_is_sent_against_the_synced_version() {
    let local = profile("b");
    assert_eq!(
        reconcile(Some(&local), None, Some(&synced("a", 3))),
        [push("b", Some(3))]
    );
    let echo = present("a", 3);
    assert_eq!(
        reconcile(Some(&local), Some(&echo), Some(&synced("a", 3))),
        [push("b", Some(3))]
    );
}

#[test]
fn equal_edits_on_both_sides_are_marked_synced_without_a_conflict() {
    let local = profile("c");
    let actions = reconcile(Some(&local), Some(&present("c", 5)), Some(&synced("a", 3)));
    assert_eq!(actions, [marked("c", 5)]);
}

#[test]
fn different_edits_on_both_sides_apply_the_remote_and_report_the_overwrite() {
    let local = profile("b");
    let actions = reconcile(Some(&local), Some(&present("c", 5)), Some(&synced("a", 3)));
    assert_eq!(
        actions,
        [
            save("c", 5, Some("b")),
            ReplicaAction::Overwritten {
                id: id(),
                discarded: Box::new(profile("b")),
            },
        ]
    );
}

#[test]
fn a_new_local_profile_without_history_is_sent_as_new() {
    let local = profile("a");
    assert_eq!(reconcile(Some(&local), None, None), [push("a", None)]);
}

#[test]
fn a_new_remote_profile_without_history_is_applied_as_new() {
    let actions = reconcile(None, Some(&present("a", 2)), None);
    assert_eq!(actions, [save("a", 2, None)]);
}

#[test]
fn a_local_and_a_remote_profile_without_history_are_reconciled_like_two_edits() {
    let local = profile("a");
    assert_eq!(
        reconcile(Some(&local), Some(&present("a", 2)), None),
        [marked("a", 2)]
    );
    assert_eq!(
        reconcile(Some(&profile("b")), Some(&present("a", 2)), None),
        [
            save("a", 2, Some("b")),
            ReplicaAction::Overwritten {
                id: id(),
                discarded: Box::new(profile("b")),
            },
        ]
    );
}

#[test]
fn a_local_deletion_is_sent_when_the_remote_is_unchanged() {
    let actions = reconcile(None, None, Some(&synced("a", 3)));
    assert_eq!(
        actions,
        [ReplicaAction::PushDelete {
            scope: ReplicaScope::Personal,
            id: id(),
            expected: version(3),
        }]
    );
}

#[test]
fn a_remote_edit_restores_a_locally_deleted_profile() {
    let actions = reconcile(None, Some(&present("b", 4)), Some(&synced("a", 3)));
    assert_eq!(actions, [save("b", 4, None)]);
}

#[test]
fn a_remote_tombstone_deletes_an_unchanged_local_profile() {
    let local = profile("a");
    let tombstone = remote(RemoteBody::Deleted, 4);
    let actions = reconcile(Some(&local), Some(&tombstone), Some(&synced("a", 3)));
    assert_eq!(
        actions,
        [ReplicaAction::DeleteLocal {
            id: id(),
            expected: Box::new(profile("a")),
        }]
    );
}

#[test]
fn a_local_edit_is_sent_over_a_remote_tombstone() {
    let local = profile("b");
    let tombstone = remote(RemoteBody::Deleted, 4);
    let actions = reconcile(Some(&local), Some(&tombstone), Some(&synced("a", 3)));
    assert_eq!(actions, [push("b", Some(4))]);
}

#[test]
fn deleting_on_both_sides_forgets_the_agreement() {
    let tombstone = remote(RemoteBody::Deleted, 4);
    let actions = reconcile(None, Some(&tombstone), Some(&synced("a", 3)));
    assert_eq!(actions, [ReplicaAction::Forget { id: id() }]);
}

#[test]
fn an_unrecognized_remote_object_never_touches_the_local_profile() {
    let unreadable = remote(RemoteBody::Unrecognized, 4);
    let expected = [ReplicaAction::NeedsNewerClient { id: id() }];
    let local = profile("b");
    let history = synced("a", 3);
    assert_eq!(
        reconcile(Some(&local), Some(&unreadable), Some(&history)),
        expected
    );
    assert_eq!(reconcile(Some(&local), Some(&unreadable), None), expected);
    assert_eq!(reconcile(None, Some(&unreadable), Some(&history)), expected);
    assert_eq!(reconcile(None, Some(&unreadable), None), expected);
}

#[test]
fn a_remote_tombstone_without_history_is_ignored_unless_a_local_profile_exists() {
    let tombstone = remote(RemoteBody::Deleted, 4);
    assert_eq!(reconcile(None, Some(&tombstone), None), []);
    let local = profile("a");
    assert_eq!(
        reconcile(Some(&local), Some(&tombstone), None),
        [push("a", Some(4))]
    );
}

fn profiles_of(items: &[Profile]) -> Profiles {
    let mut profiles = Profiles::default();
    for item in items {
        profiles.upsert(item.clone());
    }
    profiles
}

#[test]
fn a_plan_covers_every_profile_known_to_any_side_once() {
    let other = Profile {
        id: profile_id_for("other"),
        ..profile("other")
    };
    let third = Profile {
        id: profile_id_for("third"),
        ..profile("third")
    };
    let local = profiles_of(&[profile("a"), other.clone()]);
    let mut state = ReplicaState::default();
    state.agree(id(), synced("a", 3));
    state.agree(
        third.id,
        SyncedEntry {
            scope: ReplicaScope::Personal,
            version: version(2),
            profile: third.clone(),
        },
    );
    let change = RemoteProfile {
        scope: ReplicaScope::Personal,
        id: id(),
        version: version(4),
        body: RemoteBody::Present(Box::new(profile("b"))),
    };
    let actions = plan(&local, &[change], &state);
    let ids: Vec<_> = actions.iter().map(ReplicaAction::id).collect();
    assert_eq!(ids, [id(), other.id, third.id]);
    assert!(matches!(actions[0], ReplicaAction::SaveLocal { .. }));
    assert!(matches!(
        actions[1],
        ReplicaAction::Push { expected: None, .. }
    ));
    assert!(matches!(actions[2], ReplicaAction::PushDelete { .. }));
}

#[test]
fn a_plan_leaves_an_unrecognized_profile_alone_until_a_readable_version_arrives() {
    let local = profiles_of(&[profile("edited")]);
    let mut state = ReplicaState::default();
    state.agree(id(), synced("a", 3));
    state.mark_unrecognized(id());
    assert_eq!(plan(&local, &[], &state), []);
    let readable = present("b", 6);
    let actions = plan(&local, &[readable], &state);
    assert_eq!(actions.len(), 2);
    assert!(matches!(actions[1], ReplicaAction::Overwritten { .. }));
}

#[test]
fn replica_state_survives_serialization_with_its_unrecognized_ids() {
    let mut state = ReplicaState::default();
    state.agree(id(), synced("a", 3));
    state.advance(ReplicaCursor::new("7"));
    state.mark_unrecognized(profile_id_for("newer"));
    let text = serde_json::to_string(&state).unwrap();
    let restored: ReplicaState = serde_json::from_str(&text).unwrap();
    assert_eq!(restored, state);
    assert!(restored.is_unrecognized(profile_id_for("newer")));
    assert_eq!(restored.cursor().map(ReplicaCursor::as_str), Some("7"));
}

#[test]
fn agreeing_or_forgetting_a_profile_makes_it_readable_again() {
    let mut state = ReplicaState::default();
    state.mark_unrecognized(id());
    assert!(state.has_unrecognized());
    state.agree(id(), synced("a", 3));
    assert!(!state.is_unrecognized(id()));
    state.mark_unrecognized(id());
    state.forget(id());
    assert!(!state.has_unrecognized() && state.entry(id()).is_none());
}

fn group() -> ReplicaScope {
    ReplicaScope::Organization("acme".into())
}

fn synced_in(scope: ReplicaScope, name: &str, at: u64) -> SyncedEntry {
    SyncedEntry {
        scope,
        ..synced(name, at)
    }
}

fn push_to(scope: ReplicaScope, name: &str, expected: Option<u64>) -> ReplicaAction {
    ReplicaAction::Push {
        scope,
        profile: Box::new(profile(name)),
        expected: expected.map(version),
    }
}

fn delete_in(scope: ReplicaScope, at: u64) -> ReplicaAction {
    ReplicaAction::PushDelete {
        scope,
        id: id(),
        expected: version(at),
    }
}

#[test]
fn a_new_profile_placed_in_a_group_is_created_there() {
    let local = profile("a");
    assert_eq!(
        reconcile_placed(Some(&local), None, None, Some(&group())),
        [push_to(group(), "a", None)]
    );
}

#[test]
fn a_placement_over_a_remote_tombstone_creates_in_the_new_scope_without_its_version() {
    let local = profile("a");
    let tombstone = remote(RemoteBody::Deleted, 4);
    assert_eq!(
        reconcile_placed(Some(&local), Some(&tombstone), None, Some(&group())),
        [push_to(group(), "a", None)]
    );
    assert_eq!(
        reconcile_placed(
            Some(&local),
            Some(&tombstone),
            None,
            Some(&ReplicaScope::Personal)
        ),
        [push("a", Some(4))]
    );
}

#[test]
fn moving_a_profile_deletes_it_where_it_was_agreed_and_creates_it_in_the_new_scope() {
    let local = profile("a");
    let history = synced("a", 3);
    assert_eq!(
        reconcile_placed(Some(&local), None, Some(&history), Some(&group())),
        [
            delete_in(ReplicaScope::Personal, 3),
            push_to(group(), "a", None)
        ]
    );
    let edited = profile("b");
    assert_eq!(
        reconcile_placed(Some(&edited), None, Some(&history), Some(&group())),
        [
            delete_in(ReplicaScope::Personal, 3),
            push_to(group(), "b", None)
        ]
    );
}

#[test]
fn a_move_out_of_a_group_deletes_in_that_group_first() {
    let local = profile("a");
    let history = synced_in(group(), "a", 3);
    assert_eq!(
        reconcile_placed(
            Some(&local),
            None,
            Some(&history),
            Some(&ReplicaScope::Personal)
        ),
        [
            delete_in(group(), 3),
            push_to(ReplicaScope::Personal, "a", None)
        ]
    );
}

#[test]
fn a_placement_equal_to_the_agreed_scope_moves_nothing() {
    let local = profile("a");
    let history = synced_in(group(), "a", 3);
    assert_eq!(
        reconcile_placed(Some(&local), None, Some(&history), Some(&group())),
        []
    );
    let edited = profile("b");
    assert_eq!(
        reconcile_placed(Some(&edited), None, Some(&history), Some(&group())),
        [push_to(group(), "b", Some(3))]
    );
}

#[test]
fn a_remote_edit_wins_over_a_move_and_a_remote_tombstone_over_an_unedited_one() {
    let local = profile("a");
    let history = synced("a", 3);
    assert_eq!(
        reconcile_placed(
            Some(&local),
            Some(&present("c", 5)),
            Some(&history),
            Some(&group())
        ),
        [save("c", 5, Some("a"))]
    );
    let tombstone = remote(RemoteBody::Deleted, 4);
    assert_eq!(
        reconcile_placed(
            Some(&local),
            Some(&tombstone),
            Some(&history),
            Some(&group())
        ),
        [ReplicaAction::DeleteLocal {
            id: id(),
            expected: Box::new(profile("a")),
        }]
    );
}

#[test]
fn an_edited_profile_over_a_tombstone_is_moved_by_creating_only() {
    let edited = profile("b");
    let tombstone = remote(RemoteBody::Deleted, 4);
    assert_eq!(
        reconcile_placed(
            Some(&edited),
            Some(&tombstone),
            Some(&synced("a", 3)),
            Some(&group())
        ),
        [push_to(group(), "b", None)]
    );
}

#[test]
fn a_locally_deleted_profile_is_deleted_remotely_whatever_was_asked_for_it() {
    assert_eq!(
        reconcile_placed(None, None, Some(&synced("a", 3)), Some(&group())),
        [delete_in(ReplicaScope::Personal, 3)]
    );
}

#[test]
fn a_plan_moves_a_placed_profile_and_drops_a_placement_nothing_holds_any_more() {
    let local = profiles_of(&[profile("a")]);
    let gone = profile_id_for("gone");
    let mut state = ReplicaState::default();
    state.agree(id(), synced("a", 3));
    state.place(id(), group());
    state.place(gone, group());
    let actions = plan(&local, &[], &state);
    assert_eq!(
        actions,
        [
            delete_in(ReplicaScope::Personal, 3),
            push_to(group(), "a", None),
            ReplicaAction::Forget { id: gone },
        ]
    );
}

#[test]
fn a_placement_lasts_until_the_profile_is_agreed_or_forgotten() {
    let mut state = ReplicaState::default();
    state.place(id(), group());
    assert_eq!(state.placement(id()), Some(&group()));
    state.agree(id(), synced_in(group(), "a", 3));
    assert_eq!(state.placement(id()), None);
    state.place(id(), ReplicaScope::Personal);
    state.forget(id());
    assert_eq!(state.placement(id()), None);
    state.place(id(), group());
    state.unplace(id());
    assert_eq!(state.placements().count(), 0);
}

#[test]
fn a_placement_survives_serialization() {
    let mut state = ReplicaState::default();
    state.place(id(), group());
    let restored: ReplicaState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert_eq!(restored.placement(id()), Some(&group()));
}

fn remote_in(scope: ReplicaScope, body: RemoteBody, at: u64) -> RemoteProfile {
    RemoteProfile {
        scope,
        ..remote(body, at)
    }
}

#[test]
fn a_profile_another_device_moved_arrives_as_a_move_not_as_a_deletion() {
    let local = profiles_of(&[profile("a")]);
    let mut state = ReplicaState::default();
    state.agree(id(), synced("a", 3));
    let tombstone = remote_in(ReplicaScope::Personal, RemoteBody::Deleted, 4);
    let created = remote_in(group(), RemoteBody::Present(Box::new(profile("a"))), 2);
    for changes in [
        [tombstone.clone(), created.clone()],
        [created.clone(), tombstone.clone()],
    ] {
        assert_eq!(
            plan(&local, &changes, &state),
            [ReplicaAction::MarkSynced {
                scope: group(),
                version: version(2),
                profile: Box::new(profile("a")),
            }]
        );
    }
}

#[test]
fn a_feed_entry_older_than_the_agreed_version_never_reverts_the_local_profile() {
    let synced = synced("current", 5);
    let stale = [
        present("old", 3),
        remote(RemoteBody::Deleted, 4),
        remote(RemoteBody::Unrecognized, 2),
    ];
    for change in &stale {
        assert_eq!(
            reconcile(Some(&profile("current")), Some(change), Some(&synced)),
            []
        );
    }
}

#[test]
fn our_own_move_read_back_from_the_feed_changes_nothing() {
    let local = profiles_of(&[profile("a")]);
    let mut state = ReplicaState::default();
    state.agree(id(), synced_in(group(), "a", 2));
    let tombstone = remote_in(ReplicaScope::Personal, RemoteBody::Deleted, 4);
    let created = remote_in(group(), RemoteBody::Present(Box::new(profile("a"))), 2);
    assert_eq!(plan(&local, &[created, tombstone], &state), []);
}

#[test]
fn of_two_live_copies_the_one_in_the_agreed_scope_stands() {
    let local = profiles_of(&[profile("a")]);
    let mut state = ReplicaState::default();
    state.agree(id(), synced("a", 3));
    let home = remote_in(
        ReplicaScope::Personal,
        RemoteBody::Present(Box::new(profile("b"))),
        5,
    );
    let stray = remote_in(group(), RemoteBody::Present(Box::new(profile("c"))), 2);
    let actions = plan(&local, &[home, stray], &state);
    assert_eq!(actions, [save("b", 5, Some("a"))]);
}

#[test]
fn a_local_edit_is_still_sent_against_the_agreed_version_beside_a_stale_feed_entry() {
    assert_eq!(
        reconcile(
            Some(&profile("edited")),
            Some(&present("old", 3)),
            Some(&synced("current", 5)),
        ),
        [push("edited", Some(5))]
    );
}

#[test]
fn a_plan_ignores_stale_entries_and_keeps_an_unreadable_profile_unreadable() {
    let local = profiles_of(&[profile("current")]);
    let mut state = ReplicaState::default();
    state.agree(id(), synced("current", 5));
    assert_eq!(plan(&local, &[present("old", 3)], &state), []);
    state.mark_unrecognized(id());
    assert_eq!(plan(&local, &[present("old", 3)], &state), []);
    assert!(state.is_unrecognized(id()));
}

fn using(name: &str, credential: CredentialId) -> Profile {
    Profile {
        id: profile_id_for(name),
        credential: Some(credential),
        ..profile(name)
    }
}

fn acme() -> ReplicaScope {
    ReplicaScope::Organization("acme".into())
}

fn secret(byte: u8) -> CredentialId {
    CredentialId::from_bytes([byte; 16])
}

fn agreed_in(state: &mut ReplicaState, scope: ReplicaScope, profile: &Profile) {
    state.agree(
        profile.id,
        SyncedEntry {
            scope,
            version: version(1),
            profile: profile.clone(),
        },
    );
}

#[test]
fn a_secret_another_profile_in_the_same_space_uses_is_shared() {
    let (moving, staying) = (using("moving", secret(1)), using("staying", secret(1)));
    let local = profiles_of(&[moving.clone(), staying.clone()]);
    let mut state = ReplicaState::default();
    agreed_in(&mut state, ReplicaScope::Personal, &moving);
    agreed_in(&mut state, ReplicaScope::Personal, &staying);
    assert_eq!(
        rules::shared_secrets(
            &local,
            &state,
            moving.id,
            &moving.credentials(),
            &ReplicaScope::Personal
        ),
        [secret(1)]
    );
}

#[test]
fn a_secret_only_the_moving_profile_uses_is_not_shared() {
    let (moving, other) = (using("moving", secret(1)), using("other", secret(2)));
    let local = profiles_of(&[moving.clone(), other.clone()]);
    let mut state = ReplicaState::default();
    agreed_in(&mut state, ReplicaScope::Personal, &moving);
    agreed_in(&mut state, ReplicaScope::Personal, &other);
    assert!(
        rules::shared_secrets(
            &local,
            &state,
            moving.id,
            &moving.credentials(),
            &ReplicaScope::Personal
        )
        .is_empty()
    );
}

#[test]
fn a_profile_kept_in_another_space_or_leaving_does_not_share_the_source() {
    let (moving, elsewhere, leaving) = (
        using("moving", secret(1)),
        using("elsewhere", secret(1)),
        using("leaving", secret(1)),
    );
    let local = profiles_of(&[moving.clone(), elsewhere.clone(), leaving.clone()]);
    let mut state = ReplicaState::default();
    agreed_in(&mut state, ReplicaScope::Personal, &moving);
    agreed_in(&mut state, acme(), &elsewhere);
    agreed_in(&mut state, ReplicaScope::Personal, &leaving);
    state.place(leaving.id, acme());
    assert!(
        rules::shared_secrets(
            &local,
            &state,
            moving.id,
            &moving.credentials(),
            &ReplicaScope::Personal
        )
        .is_empty()
    );
}

#[test]
fn a_profile_never_sent_counts_as_staying_personal() {
    let (moving, fresh) = (using("moving", secret(1)), using("fresh", secret(1)));
    let local = profiles_of(&[moving.clone(), fresh]);
    let mut state = ReplicaState::default();
    agreed_in(&mut state, acme(), &moving);
    assert_eq!(
        rules::shared_secrets(
            &local,
            &state,
            moving.id,
            &moving.credentials(),
            &ReplicaScope::Personal
        ),
        [secret(1)]
    );
}
