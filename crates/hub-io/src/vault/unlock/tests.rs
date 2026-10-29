use std::sync::Arc;

use super::*;
use crate::account::{
    Slot,
    operations::AccountOperations,
    testing::{PASSWORD, native_operations, record, signed_in, unlock},
};
use crate::store::memory::MemoryHub;

fn locked_device(
    hub: &Arc<MemoryHub>,
) -> (
    Unlocker,
    Arc<Slot>,
    tempfile::TempDir,
    Arc<AccountOperations>,
) {
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = signed_in(dir.path());
    hub.set_keyset(Some(record));
    let operations = native_operations(&slot, hub.clone());
    slot.lock();
    let unlocker = Unlocker::new(operations.membership().unwrap());
    (unlocker, slot, dir, operations)
}

#[tokio::test]
async fn the_password_unlocks_with_the_hub_reachable() {
    let hub = Arc::new(MemoryHub::default());
    let (unlocker, slot, _dir, _operations) = locked_device(&hub);
    unlocker.unlock(Secret::new(PASSWORD)).await.unwrap();
    assert!(slot.open().is_some());
}

#[tokio::test]
async fn the_password_unlocks_with_the_hub_unreachable() {
    let hub = Arc::new(MemoryHub::default());
    let (unlocker, slot, _dir, _operations) = locked_device(&hub);
    hub.set_offline(true);
    unlocker.unlock(Secret::new(PASSWORD)).await.unwrap();
    assert!(slot.open().is_some());
}

#[tokio::test]
async fn a_wrong_password_stays_locked_and_says_so() {
    let hub = Arc::new(MemoryHub::default());
    let (unlocker, slot, _dir, _operations) = locked_device(&hub);
    let result = unlocker.unlock(Secret::new("another password")).await;
    assert!(matches!(result, Err(VaultError::Authentication)));
    assert!(slot.open().is_none());
}

#[tokio::test]
async fn a_password_that_is_too_short_is_not_tried() {
    let hub = Arc::new(MemoryHub::default());
    let (unlocker, _slot, _dir, _operations) = locked_device(&hub);
    assert!(matches!(
        unlocker.unlock(Secret::new("short")).await,
        Err(VaultError::Password)
    ));
}

#[tokio::test]
async fn a_keyset_changed_on_another_device_is_taken_from_the_hub() {
    let hub = Arc::new(MemoryHub::default());
    let (unlocker, slot, _dir, _operations) = locked_device(&hub);
    // Another device recovered the account: the hub now holds a newer record that the same
    // password opens.
    let (mut newer, _) = record(&unlock(PASSWORD));
    newer.version = 5;
    hub.set_keyset(Some(newer.clone()));
    unlocker.unlock(Secret::new(PASSWORD)).await.unwrap();
    assert_eq!(slot.read_cache(|c| c.unwrap().keyset.version), 5);
}

#[tokio::test]
async fn an_account_without_cached_keys_has_nothing_to_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let accounts = crate::account::Accounts::new(dir.path().to_path_buf());
    let slot = accounts.slot(&crate::account::testing::hub()).unwrap();
    let operations = native_operations(&slot, Arc::new(MemoryHub::default()));
    let unlocker = Unlocker::new(operations.membership().unwrap());
    assert!(matches!(
        unlocker.unlock(Secret::new(PASSWORD)).await,
        Err(VaultError::NotFound)
    ));
}
