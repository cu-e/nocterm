use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nocterm_session::Target;

use super::*;
use crate::account::testing::NativeVault;
use crate::account::testing::{PASSWORD, signed_in};
use crate::store::memory::MemoryHub;
mod refresh;
mod source_binding;

fn vault() -> (NativeVault, Arc<MemoryHub>, tempfile::TempDir) {
    let hub = Arc::new(MemoryHub::default());
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = signed_in(dir.path());
    hub.set_keyset(Some(record));
    (NativeVault::new(slot, hub.clone()), hub, dir)
}

fn binding() -> CredentialBinding {
    CredentialBinding::Password {
        target: Target::new("ann", "example.org", 22),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_vault_reports_its_status_as_the_account_keys_change() {
    let (vault, _hub, _dir) = vault();
    assert_eq!(vault.status(), VaultStatus::Unlocked);
    let seen = vault.revision();
    vault.lock();
    assert_eq!(vault.status(), VaultStatus::Locked);
    assert!(vault.revision() > seen);
    assert!(vault.exists() && !vault.is_unlocked());
    vault.unlock(Secret::new(PASSWORD)).await.unwrap();
    assert_eq!(vault.status(), VaultStatus::Unlocked);
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_for_a_change_ends_when_the_vault_locks() {
    let (vault, _hub, _dir) = vault();
    let seen = vault.revision();
    let changed = vault.changed(seen);
    vault.lock();
    let now = tokio::time::timeout(Duration::from_secs(5), changed)
        .await
        .unwrap();
    assert!(now > seen);
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_for_unlock_ends_when_the_password_is_accepted() {
    let (vault, _hub, _dir) = vault();
    vault.lock();
    let waiting = vault.wait_unlocked();
    vault.unlock(Secret::new(PASSWORD)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vault_on_a_hub_has_no_password_of_its_own_to_create_or_change() {
    let (vault, _hub, _dir) = vault();
    assert!(matches!(
        vault.create(Secret::new("a password")).await,
        Err(VaultError::Unsupported)
    ));
    assert!(matches!(
        vault.change_password(Secret::new("a password")).await,
        Err(VaultError::Unsupported)
    ));
    assert!(matches!(
        vault.enable_device_unlock().await,
        Err(VaultError::Unsupported)
    ));
    assert!(matches!(
        vault.unlock_with_device().await,
        Err(VaultError::Unsupported)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn credentials_are_saved_and_read_through_the_port() {
    let (vault, _hub, _dir) = vault();
    let id = vault
        .put(None, "Work".into(), binding(), Secret::new("pw"))
        .await
        .unwrap();
    assert_eq!(vault.list().await.unwrap().len(), 1);
    assert_eq!(vault.get(id, binding()).await.unwrap().expose(), "pw");
    vault.delete(id).await.unwrap();
    assert!(vault.list().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_a_write_is_refused_as_offline() {
    let (vault, hub, _dir) = vault();
    hub.set_offline(true);
    assert!(matches!(
        vault
            .put(None, "Work".into(), binding(), Secret::new("pw"))
            .await,
        Err(VaultError::Offline)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_locked_vault_does_not_answer() {
    let (vault, _hub, _dir) = vault();
    vault.lock();
    assert!(matches!(vault.list().await, Err(VaultError::Locked)));
    let other = CredentialBinding::KeyPassphrase {
        path: PathBuf::from("/k"),
    };
    assert!(matches!(
        vault.get(CredentialId::from_bytes([1; 16]), other).await,
        Err(VaultError::Locked)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_vault_locks_itself_after_a_quiet_period() {
    let (vault, _hub, _dir) = vault();
    vault.set_auto_lock(Duration::from_millis(60));
    assert_eq!(vault.status(), VaultStatus::Unlocked);
    tokio::time::timeout(Duration::from_secs(5), async {
        while vault.status() == VaultStatus::Unlocked {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(vault.status(), VaultStatus::Locked);
}
