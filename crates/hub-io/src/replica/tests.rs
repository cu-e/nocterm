use std::time::Duration;

use nocterm_connections_model::replica::{RemoteBody, ReplicaAccess};
use nocterm_connections_model::test_support::profile_id_for;

use super::*;
use crate::account::testing::{second_device, signed_in};
use crate::store::memory::MemoryHub;

fn profile(name: &str) -> Profile {
    serde_json::from_value(serde_json::json!({
        "id": profile_id_for(name),
        "name": name,
        "host": "example.org",
        "user": "ann",
    }))
    .unwrap()
}

struct Device {
    replica: HubProfileReplica,
    slot: Arc<Slot>,
    _dir: tempfile::TempDir,
}

fn device(hub: &Arc<MemoryHub>) -> (Device, nocterm_hub_proto::v1::AccountKeyset) {
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = signed_in(dir.path());
    let ledger = Arc::new(Ledger::new(Arc::clone(&slot), hub.clone()));
    let replica = HubProfileReplica::new(Handle::current(), Arc::clone(&slot), hub.clone(), ledger);
    (
        Device {
            replica,
            slot,
            _dir: dir,
        },
        record,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_profile_put_on_one_device_arrives_on_another() {
    let hub = Arc::new(MemoryHub::default());
    let (first, record) = device(&hub);
    let web = profile("web");
    let version = first
        .replica
        .put(ReplicaScope::Personal, web.clone(), None)
        .await
        .unwrap();
    assert_eq!(version, ReplicaVersion::new(1));

    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot) = second_device(dir.path(), &record);
    let ledger = Arc::new(Ledger::new(Arc::clone(&slot), hub.clone()));
    let second = HubProfileReplica::new(Handle::current(), slot, hub.clone(), ledger);
    let page = second.changes(None).await.unwrap();
    assert!(!page.more);
    assert_eq!(page.changes.len(), 1);
    assert_eq!(page.changes[0].body, RemoteBody::Present(Box::new(web)));
    // Reading again from the returned cursor finds nothing new.
    let next = second.changes(Some(page.cursor)).await.unwrap();
    assert!(next.changes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_write_conflicts_and_a_delete_is_a_tombstone() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let web = profile("web");
    let v1 = device
        .replica
        .put(ReplicaScope::Personal, web.clone(), None)
        .await
        .unwrap();
    let again = device
        .replica
        .put(ReplicaScope::Personal, web.clone(), None)
        .await;
    assert!(matches!(again, Err(ReplicaError::Conflict { id }) if id == web.id));
    let v2 = device
        .replica
        .put(ReplicaScope::Personal, web.clone(), Some(v1))
        .await
        .unwrap();
    let v3 = device
        .replica
        .delete(ReplicaScope::Personal, web.id, v2)
        .await
        .unwrap();
    assert!(v3 > v2);
    let page = device.replica.changes(None).await.unwrap();
    assert_eq!(page.changes.len(), 1);
    assert_eq!(page.changes[0].body, RemoteBody::Deleted);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_locked_or_unreachable_replica_is_unavailable_not_failed() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    hub.set_offline(true);
    assert!(matches!(
        device.replica.changes(None).await,
        Err(ReplicaError::Unavailable)
    ));
    hub.set_offline(false);
    device.slot.lock();
    assert!(matches!(
        device.replica.changes(None).await,
        Err(ReplicaError::Unavailable)
    ));
    assert!(matches!(
        device
            .replica
            .put(ReplicaScope::Personal, profile("web"), None)
            .await,
        Err(ReplicaError::Unavailable)
    ));
}

fn acme() -> ReplicaScope {
    ReplicaScope::Organization("acme".into())
}

const ACME_PROFILES: &str = "acme-profiles";

/// An organization whose administrator granted the account the key of its profiles; the
/// returned key is what the other members seal with.
fn acme_group(
    hub: &Arc<MemoryHub>,
    record: &nocterm_hub_proto::v1::AccountKeyset,
    role: nocterm_hub_proto::v1::Role,
) -> nocterm_keys::CollectionKey {
    let (key, envelope) = crate::account::testing::granted(record, ACME_PROFILES);
    hub.add_group(PROFILES_KIND, "acme", ACME_PROFILES, envelope);
    hub.set_memberships(&[("acme", "Acme", role)]);
    key
}

/// Another member stores `profile` in the group.
fn member_writes(hub: &MemoryHub, key: &nocterm_keys::CollectionKey, profile: &Profile) {
    let id = profile.id.to_string();
    let plain = wire::encode(profile).unwrap();
    let sealed = key.seal_object(ACME_PROFILES, &id, KIND, &plain).unwrap();
    hub.write_as_member(ACME_PROFILES, &id, KIND, sealed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_groups_profiles_arrive_in_its_scope_next_to_the_personal_ones() {
    use nocterm_hub_proto::v1::Role;
    let hub = Arc::new(MemoryHub::default());
    let (device, record) = device(&hub);
    let key = acme_group(&hub, &record, Role::Member);
    let (mine, team) = (profile("mine"), profile("team"));
    device
        .replica
        .put(ReplicaScope::Personal, mine.clone(), None)
        .await
        .unwrap();
    member_writes(&hub, &key, &team);
    let page = device.replica.changes(None).await.unwrap();
    let scopes: Vec<_> = page
        .changes
        .iter()
        .map(|change| (change.id, change.scope.clone()))
        .collect();
    assert_eq!(
        scopes,
        [(mine.id, ReplicaScope::Personal), (team.id, acme())]
    );
    assert_eq!(
        page.changes[1].body,
        RemoteBody::Present(Box::new(team.clone()))
    );
    let again = device.replica.changes(Some(page.cursor)).await.unwrap();
    assert!(again.changes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_spaces_are_the_personal_one_and_each_group_with_the_members_access() {
    use nocterm_hub_proto::v1::Role;
    let hub = Arc::new(MemoryHub::default());
    let (device, record) = device(&hub);
    assert_eq!(device.replica.spaces(), []);
    acme_group(&hub, &record, Role::Guest);
    device.replica.changes(None).await.unwrap();
    let spaces = device.replica.spaces();
    assert_eq!(
        spaces
            .iter()
            .map(|space| (space.scope.clone(), space.name.as_str(), space.access))
            .collect::<Vec<_>>(),
        [
            (ReplicaScope::Personal, "Personal", ReplicaAccess::Write),
            (acme(), "Acme", ReplicaAccess::Read),
        ]
    );
    hub.set_memberships(&[("acme", "Acme", Role::Admin)]);
    device.replica.changes(None).await.unwrap();
    assert_eq!(device.replica.spaces()[1].access, ReplicaAccess::Write);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_without_a_granted_key_is_not_a_space_and_refuses_writes() {
    use nocterm_hub_proto::v1::Role;
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    hub.add_foreign(PROFILES_KIND, "acme");
    hub.set_memberships(&[("acme", "Acme", Role::Member)]);
    device.replica.changes(None).await.unwrap();
    assert_eq!(device.replica.spaces().len(), 1);
    let web = profile("web");
    let result = device.replica.put(acme(), web.clone(), None).await;
    assert!(matches!(result, Err(ReplicaError::Forbidden { id }) if id == web.id));
    let nowhere = device
        .replica
        .put(ReplicaScope::Organization("none".into()), web.clone(), None)
        .await;
    assert!(matches!(nowhere, Err(ReplicaError::Forbidden { id }) if id == web.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_writes_into_the_group_sealed_with_the_groups_key() {
    use nocterm_hub_proto::v1::Role;
    let hub = Arc::new(MemoryHub::default());
    let (device, record) = device(&hub);
    let key = acme_group(&hub, &record, Role::Member);
    let web = profile("web");
    let v1 = device.replica.put(acme(), web.clone(), None).await.unwrap();
    assert_eq!(hub.objects(ACME_PROFILES).len(), 1);
    assert!(
        hub.collections(PROFILES_KIND)
            .await
            .unwrap()
            .iter()
            .all(|c| c.personal == (c.collection_id != ACME_PROFILES))
    );
    let stored = &hub.objects(ACME_PROFILES)[0];
    let opened = key
        .open_object(
            ACME_PROFILES,
            &stored.object_id,
            &stored.kind,
            &stored.ciphertext,
        )
        .unwrap();
    assert!(String::from_utf8_lossy(&opened).contains("\"web\""));
    let v2 = device
        .replica
        .put(acme(), profile("web"), Some(v1))
        .await
        .unwrap();
    device.replica.delete(acme(), web.id, v2).await.unwrap();
    assert!(hub.objects(ACME_PROFILES)[0].deleted);
    assert!(
        hub.collections(PROFILES_KIND)
            .await
            .unwrap()
            .iter()
            .all(|c| c.collection_id == ACME_PROFILES || c.personal),
        "nothing was created for the group"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guest_write_is_forbidden_and_leaves_the_group_untouched() {
    use nocterm_hub_proto::v1::Role;
    let hub = Arc::new(MemoryHub::default());
    let (device, record) = device(&hub);
    let key = acme_group(&hub, &record, Role::Guest);
    let team = profile("team");
    member_writes(&hub, &key, &team);
    hub.refuse_writes_to(ACME_PROFILES);
    let version = device.replica.changes(None).await.unwrap().changes[0].version;
    let edited = device
        .replica
        .put(acme(), profile("team"), Some(version))
        .await;
    assert!(matches!(edited, Err(ReplicaError::Forbidden { id }) if id == team.id));
    let deleted = device.replica.delete(acme(), team.id, version).await;
    assert!(matches!(deleted, Err(ReplicaError::Forbidden { id }) if id == team.id));
    assert!(!hub.objects(ACME_PROFILES)[0].deleted);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_key_that_does_not_open_leaves_the_personal_profiles_readable() {
    use nocterm_hub_proto::v1::Role;
    let hub = Arc::new(MemoryHub::default());
    let (device, record) = device(&hub);
    acme_group(&hub, &record, Role::Member);
    hub.tamper_key(ACME_PROFILES, vec![9; 80]);
    let mine = profile("mine");
    device
        .replica
        .put(ReplicaScope::Personal, mine.clone(), None)
        .await
        .unwrap();
    let page = device.replica.changes(None).await.unwrap();
    assert_eq!(page.changes.len(), 1);
    assert_eq!(page.changes[0].scope, ReplicaScope::Personal);
    assert_eq!(device.replica.spaces().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_profile_moved_to_a_group_is_deleted_personally_then_created_there() {
    use nocterm_hub_proto::v1::Role;
    let hub = Arc::new(MemoryHub::default());
    let (device, record) = device(&hub);
    let key = acme_group(&hub, &record, Role::Member);
    let web = profile("web");
    let v1 = device
        .replica
        .put(ReplicaScope::Personal, web.clone(), None)
        .await
        .unwrap();
    device
        .replica
        .delete(ReplicaScope::Personal, web.id, v1)
        .await
        .unwrap();
    device.replica.put(acme(), web.clone(), None).await.unwrap();
    let page = device.replica.changes(None).await.unwrap();
    let bodies: Vec<_> = page
        .changes
        .iter()
        .map(|change| (change.scope.clone(), change.body.clone()))
        .collect();
    assert_eq!(
        bodies,
        [
            (ReplicaScope::Personal, RemoteBody::Deleted),
            (acme(), RemoteBody::Present(Box::new(web.clone()))),
        ]
    );
    let stored = &hub.objects(ACME_PROFILES)[0];
    assert!(
        key.open_object(
            ACME_PROFILES,
            &stored.object_id,
            &stored.kind,
            &stored.ciphertext
        )
        .is_ok()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_profile_the_hub_altered_stops_the_feed_with_an_error() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let web = profile("web");
    device
        .replica
        .put(ReplicaScope::Personal, web.clone(), None)
        .await
        .unwrap();
    let collection = hub.collections(PROFILES_KIND).await.unwrap().remove(0);
    hub.tamper(&collection.collection_id, &web.id.to_string(), vec![5; 64]);
    assert!(matches!(
        device.replica.changes(None).await,
        Err(ReplicaError::Other(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_watch_ticks_when_the_vault_unlocks() {
    let hub = Arc::new(MemoryHub::default());
    let (device, record) = device(&hub);
    device.slot.lock();
    let mut ticks = device.replica.watch();
    // Locked: no tick yet.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), ticks.next())
            .await
            .is_err()
    );
    let keys = crate::account::testing::open(
        &record,
        &crate::account::testing::unlock(crate::account::testing::PASSWORD),
    );
    device.slot.unlocked(keys);
    let tick = tokio::time::timeout(Duration::from_secs(5), ticks.next())
        .await
        .unwrap();
    assert_eq!(tick, Some(()));
}
