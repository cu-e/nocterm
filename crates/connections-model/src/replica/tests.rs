use super::rules::{ReplicaAction, plan, reconcile};
use super::*;
use crate::{profiles::Profiles, test_support::profile_id_for};
use nocterm_session::{Auth, Target};

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
