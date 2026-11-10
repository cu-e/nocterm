use std::sync::Arc;
use std::time::Duration;

use nocterm_connections_model::replica::RemoteBody;
use nocterm_connections_model::test_support::profile_id_for;

use super::*;
use crate::account::{
    Slot,
    operations::AccountOperations,
    testing::{native_operations, second_device, signed_in},
};
use crate::store::StoreOps as _;
use crate::store::memory::MemoryHub;
mod source_binding;

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
    _operations: Arc<AccountOperations>,
}

/// A signed-in device of the account.
fn device(hub: &Arc<MemoryHub>) -> (Device, nocterm_hub_proto::v2::Keyset) {
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = signed_in(dir.path());
    let operations = native_operations(&slot, hub.clone());
    let source = operations.current_opened_source().unwrap();
    let replica = HubProfileReplica::new(Handle::current(), operations.bind(source).unwrap());
    (
        Device {
            replica,
            slot,
            _dir: dir,
            _operations: operations,
        },
        record,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_profile_put_on_one_device_arrives_on_another() {
    let hub = Arc::new(MemoryHub::default());
    let (first, record) = device(&hub);
    let web = profile("web");
    let version = first.replica.put(web.clone(), None).await.unwrap();
    assert_eq!(version, ReplicaVersion::new(1));

    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot) = second_device(dir.path(), &record);
    let operations = native_operations(&slot, hub.clone());
    let source = operations.current_opened_source().unwrap();
    let second = HubProfileReplica::new(Handle::current(), operations.bind(source).unwrap());
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
    let v1 = device.replica.put(web.clone(), None).await.unwrap();
    let again = device.replica.put(web.clone(), None).await;
    assert!(matches!(again, Err(ReplicaError::Conflict { id }) if id == web.id));
    let v2 = device.replica.put(web.clone(), Some(v1)).await.unwrap();
    let v3 = device.replica.delete(web.id, v2).await.unwrap();
    assert!(v3 > v2);
    let page = device.replica.changes(None).await.unwrap();
    assert_eq!(page.changes.len(), 1);
    assert_eq!(page.changes[0].body, RemoteBody::Deleted);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_hub_is_unavailable_not_failed() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    hub.set_offline(true);
    assert!(matches!(
        device.replica.changes(None).await,
        Err(ReplicaError::Unavailable)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_account_keys_makes_the_original_replica_stale() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    device.slot.lock();
    assert!(matches!(
        device.replica.changes(None).await,
        Err(ReplicaError::Stale)
    ));
    assert!(matches!(
        device.replica.put(profile("web"), None).await,
        Err(ReplicaError::Stale)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_profile_the_hub_altered_stops_the_feed_with_an_error() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let web = profile("web");
    device.replica.put(web.clone(), None).await.unwrap();
    let collection = hub.collections(PROFILES_KIND).await.unwrap().remove(0);
    hub.tamper(&collection.collection_id, &web.id.to_string(), vec![5; 64]);
    assert!(matches!(
        device.replica.changes(None).await,
        Err(ReplicaError::Other(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_old_watch_ends_after_lock_and_a_fresh_capture_watches_after_unlock() {
    let hub = Arc::new(MemoryHub::default());
    let (device, record) = device(&hub);
    device.slot.lock();
    let mut old = device.replica.watch();
    assert_eq!(old.next().await, None);
    let keys = crate::account::testing::open(
        &record,
        &crate::account::testing::unlock(crate::account::testing::PASSWORD),
    );
    device.slot.unlocked(keys);
    assert_eq!(old.next().await, None);
    let source = device._operations.current_opened_source().unwrap();
    let fresh = HubProfileReplica::new(Handle::current(), device._operations.bind(source).unwrap());
    let mut ticks = fresh.watch();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), ticks.next())
            .await
            .unwrap(),
        Some(())
    );
}
