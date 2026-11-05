//! The group of an organization on a real hub, seen by two users.
//!
//! The hub needs a group the primary account belongs to as a Member or better, and a second
//! account that is a Guest of the same group. An administrator makes them with `hubctl` (create
//! the group, add both users, then `hubctl orgs confirm` for each so the group key is granted to
//! them). Nocterm itself never creates, grants or deletes a group. Variables, besides those of
//! the other tests:
//!
//! - `NOCTERM_HUB_ORG`: the id of the organization whose group both users share;
//! - `NOCTERM_HUB_GUEST_USERNAME` and `NOCTERM_HUB_GUEST_PASSWORD`: the Guest.
//!
//! Without `NOCTERM_REQUIRE_HUB` the tests pass vacuously when they are missing. The profile the
//! tests write is removed again. Each test device is new, so it confirms the administrator's key
//! on first use, as a person does after comparing its fingerprint with `hubctl`'s.

use super::{
    Hub,
    account::{Device, unique},
    hub,
};
use std::sync::Arc;

use futures::executor::block_on;
use nocterm_connections_model::{
    profiles::Profile,
    replica::{ProfileReplica, RemoteBody, ReplicaAccess, ReplicaError, ReplicaScope},
    test_support::profile_id_for,
};
use nocterm_hubs_model::HubAddress;
use nocterm_session::{Auth, Target};

struct Group {
    primary: Hub,
    guest: Hub,
    scope: ReplicaScope,
}

fn group() -> Option<Group> {
    let primary = hub()?;
    let variable = |name: &str| std::env::var(name).ok();
    let (Some(org), Some(username), Some(password)) = (
        variable("NOCTERM_HUB_ORG"),
        variable("NOCTERM_HUB_GUEST_USERNAME"),
        variable("NOCTERM_HUB_GUEST_PASSWORD"),
    ) else {
        assert!(
            std::env::var_os("NOCTERM_REQUIRE_HUB").is_none(),
            "NOCTERM_REQUIRE_HUB is set but NOCTERM_HUB_ORG, NOCTERM_HUB_GUEST_USERNAME and \
             NOCTERM_HUB_GUEST_PASSWORD are not all set"
        );
        return None;
    };
    let guest = Hub {
        address: HubAddress::parse(&primary.address.to_string()).expect("the hub address"),
        fingerprint: primary.fingerprint,
        username,
        password,
    };
    Some(Group {
        primary,
        guest,
        scope: ReplicaScope::Organization(org),
    })
}

/// The replica of a new device, with the keys of the group's administrators confirmed. The
/// first read finds the grants of keys this device has not pinned yet.
fn trusting_replica(device: &Device, hub: &Hub) -> Arc<dyn ProfileReplica> {
    let account = device.account(hub);
    let replica = account.replica().unwrap();
    block_on(replica.changes(None)).unwrap();
    let admins = account.admin_keys();
    for pending in admins.pending() {
        assert!(pending.key().replaces.is_none(), "{pending:?}");
        block_on(pending.confirm(&pending.key().fingerprint)).unwrap();
    }
    block_on(replica.changes(None)).unwrap();
    replica
}

fn profile(name: &str) -> Profile {
    Profile {
        id: profile_id_for(name),
        name: name.to_owned(),
        description: String::new(),
        options: Default::default(),
        target: Target::new("ci", "group.example", 22),
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

#[test]
fn a_member_shares_a_profile_with_the_group_and_a_guest_reads_but_cannot_change_it() {
    let Some(group) = group() else { return };
    let (member, _) = Device::sign_in(&group.primary);
    let (guest, _) = Device::sign_in(&group.guest);
    let member_replica = trusting_replica(&member, &group.primary);
    let guest_replica = trusting_replica(&guest, &group.guest);
    let shared = profile(&unique("live-group-profile"));
    let access = |replica: &dyn nocterm_connections_model::replica::ProfileReplica| {
        replica
            .spaces()
            .into_iter()
            .find(|space| space.scope == group.scope)
            .map(|space| space.access)
    };
    assert_eq!(access(&*member_replica), Some(ReplicaAccess::Write));
    assert_eq!(access(&*guest_replica), Some(ReplicaAccess::Read));

    let written = block_on(member_replica.put(group.scope.clone(), shared.clone(), None)).unwrap();
    let page = block_on(guest_replica.changes(None)).unwrap();
    let seen = page
        .changes
        .iter()
        .find(|change| change.id == shared.id)
        .expect("the guest sees the group's profile");
    assert_eq!(seen.scope, group.scope);
    assert_eq!(seen.body, RemoteBody::Present(Box::new(shared.clone())));

    let mut edited = shared.clone();
    edited.name = "changed by a guest".into();
    let refused = block_on(guest_replica.put(group.scope.clone(), edited, Some(written)));
    assert!(
        matches!(refused, Err(ReplicaError::Forbidden { id }) if id == shared.id),
        "{refused:?}"
    );
    let refused = block_on(guest_replica.delete(group.scope.clone(), shared.id, written));
    assert!(
        matches!(refused, Err(ReplicaError::Forbidden { .. })),
        "{refused:?}"
    );

    block_on(member_replica.delete(group.scope.clone(), shared.id, written)).unwrap();
}

#[test]
fn a_profile_moved_between_spaces_leaves_the_old_one() {
    let Some(group) = group() else { return };
    let (member, _) = Device::sign_in(&group.primary);
    let replica = trusting_replica(&member, &group.primary);
    let moved = profile(&unique("live-moved-profile"));

    let personal = block_on(replica.put(ReplicaScope::Personal, moved.clone(), None)).unwrap();
    // A move is a deletion in the old space and a creation in the new one.
    block_on(replica.delete(ReplicaScope::Personal, moved.id, personal)).unwrap();
    let shared = block_on(replica.put(group.scope.clone(), moved.clone(), None)).unwrap();

    let page = block_on(replica.changes(None)).unwrap();
    let live: Vec<_> = page
        .changes
        .iter()
        .filter(|change| change.id == moved.id)
        .filter(|change| matches!(change.body, RemoteBody::Present(_)))
        .collect();
    assert_eq!(live.len(), 1, "one live copy: {live:?}");
    assert_eq!(live[0].scope, group.scope);

    block_on(replica.delete(group.scope.clone(), moved.id, shared)).unwrap();
}
