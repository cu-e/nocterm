use std::time::Duration;

use futures::StreamExt as _;
use nocterm_connections_model::replica::RemoteBody;
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
    let replica = HubProfileReplica::new(Handle::current(), Arc::clone(&slot), hub.clone());
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
    let second = HubProfileReplica::new(Handle::current(), slot, hub.clone());
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

#[tokio::test(flavor = "multi_thread")]
async fn an_organization_scope_is_not_served_here() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let web = profile("web");
    let result = device
        .replica
        .put(ReplicaScope::Organization("org".into()), web.clone(), None)
        .await;
    assert!(matches!(result, Err(ReplicaError::Forbidden { id }) if id == web.id));
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
