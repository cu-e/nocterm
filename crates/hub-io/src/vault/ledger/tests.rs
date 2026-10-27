use std::path::PathBuf;
use std::sync::Arc;

use nocterm_session::Target;

use super::codec::KIND;
use super::*;
use crate::account::testing::{second_device, signed_in};
use crate::store::VAULT_KIND;
use crate::store::memory::MemoryHub;

struct Device {
    ledger: Ledger,
    slot: Arc<Slot>,
    _dir: tempfile::TempDir,
}

fn device(hub: &Arc<MemoryHub>) -> (Device, nocterm_hub_proto::v1::AccountKeyset) {
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = signed_in(dir.path());
    let ledger = Ledger::new(Arc::clone(&slot), hub.clone());
    (
        Device {
            ledger,
            slot,
            _dir: dir,
        },
        record,
    )
}

fn password_binding() -> CredentialBinding {
    CredentialBinding::Password {
        target: Target::new("ann", "example.org", 22),
    }
}

#[tokio::test]
async fn a_saved_credential_is_listed_and_read_back() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let id = device
        .ledger
        .put(None, "Work".into(), password_binding(), Secret::new("pw"))
        .await
        .unwrap();
    let listed = device.ledger.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!((listed[0].id, listed[0].label.as_str()), (id, "Work"));
    let secret = device.ledger.get(id, password_binding()).await.unwrap();
    assert_eq!(secret.expose(), "pw");
}

#[tokio::test]
async fn a_credential_does_not_answer_another_request() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let id = device
        .ledger
        .put(None, "Work".into(), password_binding(), Secret::new("pw"))
        .await
        .unwrap();
    let other = CredentialBinding::KeyPassphrase {
        path: PathBuf::from("/k"),
    };
    assert!(matches!(
        device.ledger.get(id, other).await,
        Err(VaultError::Binding)
    ));
}

#[tokio::test]
async fn a_private_key_record_round_trips() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let binding = CredentialBinding::PrivateKey {
        public_key: "ssh-ed25519 AAAA".into(),
    };
    let id = device
        .ledger
        .put(
            None,
            "key".into(),
            binding.clone(),
            Secret::new("PRIVATE KEY TEXT"),
        )
        .await
        .unwrap();
    let secret = device.ledger.get(id, binding).await.unwrap();
    assert_eq!(secret.expose(), "PRIVATE KEY TEXT");
}

#[tokio::test]
async fn the_hub_holds_only_ciphertext() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    device
        .ledger
        .put(
            None,
            "Work label".into(),
            password_binding(),
            Secret::new("hunter2hunter2"),
        )
        .await
        .unwrap();
    let collection = hub
        .collections(crate::store::VAULT_KIND)
        .await
        .unwrap()
        .remove(0);
    let objects = hub.objects(&collection.collection_id);
    assert_eq!(objects.len(), 1);
    let text = String::from_utf8_lossy(&objects[0].ciphertext).into_owned();
    assert!(!text.contains("hunter2") && !text.contains("Work label"));
}

#[tokio::test]
async fn credentials_stay_readable_offline_but_cannot_be_written() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let id = device
        .ledger
        .put(None, "Work".into(), password_binding(), Secret::new("pw"))
        .await
        .unwrap();
    hub.set_offline(true);
    assert_eq!(device.ledger.list().await.unwrap().len(), 1);
    assert_eq!(
        device
            .ledger
            .get(id, password_binding())
            .await
            .unwrap()
            .expose(),
        "pw"
    );
    assert!(matches!(
        device
            .ledger
            .put(None, "New".into(), password_binding(), Secret::new("x"))
            .await,
        Err(VaultError::Offline)
    ));
    assert!(matches!(
        device.ledger.delete(id).await,
        Err(VaultError::Offline)
    ));
}

#[tokio::test]
async fn a_credential_saved_on_one_device_is_listed_on_another() {
    let hub = Arc::new(MemoryHub::default());
    let (first, record) = device(&hub);
    let id = first
        .ledger
        .put(None, "Shared".into(), password_binding(), Secret::new("pw"))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot) = second_device(dir.path(), &record);
    let second = Ledger::new(slot, hub.clone());
    let listed = second.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(
        second.get(id, password_binding()).await.unwrap().expose(),
        "pw"
    );
}

#[tokio::test]
async fn deleting_leaves_a_tombstone_and_the_list_without_the_credential() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let id = device
        .ledger
        .put(None, "Work".into(), password_binding(), Secret::new("pw"))
        .await
        .unwrap();
    device.ledger.delete(id).await.unwrap();
    assert!(device.ledger.list().await.unwrap().is_empty());
    assert!(matches!(
        device.ledger.get(id, password_binding()).await,
        Err(VaultError::NotFound)
    ));
    assert!(matches!(
        device.ledger.delete(id).await,
        Err(VaultError::NotFound)
    ));
}

#[tokio::test]
async fn a_locked_vault_reads_and_writes_nothing() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    device.slot.lock();
    assert!(matches!(
        device.ledger.list().await,
        Err(VaultError::Locked)
    ));
    assert!(matches!(
        device
            .ledger
            .put(None, "x".into(), password_binding(), Secret::new("x"))
            .await,
        Err(VaultError::Locked)
    ));
}

#[tokio::test]
async fn a_record_the_hub_altered_is_an_error_not_a_silent_skip() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let id = device
        .ledger
        .put(None, "Work".into(), password_binding(), Secret::new("pw"))
        .await
        .unwrap();
    let collection = hub
        .collections(crate::store::VAULT_KIND)
        .await
        .unwrap()
        .remove(0);
    hub.tamper(&collection.collection_id, &id.to_string(), vec![9; 64]);
    assert!(matches!(
        device.ledger.list().await,
        Err(VaultError::Authentication)
    ));
}

/// Writes an object the way a newer client would: sealed with the right key but holding a
/// binding this client does not know.
async fn write_foreign(device: &Device, hub: &Arc<MemoryHub>, object_id: &str, text: &str) {
    let keys = device.slot.open().unwrap();
    let store = Store::new(hub.clone(), keys);
    let at = store.ensure(VAULT_KIND).await.unwrap();
    store
        .put(&at, KIND, object_id, text.as_bytes(), 0)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_record_of_an_unknown_kind_is_hidden_and_never_changed() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let foreign: CredentialId = "00000000000000000000000000000042".parse().unwrap();
    let text =
        r#"{"v":1,"label":"token","binding":{"kind":"hardware_token","slot":1},"secret":"s"}"#;
    write_foreign(&device, &hub, &foreign.to_string(), text).await;
    let before = {
        let collection = hub.collections(VAULT_KIND).await.unwrap().remove(0);
        hub.objects(&collection.collection_id)
    };

    assert!(device.ledger.list().await.unwrap().is_empty());
    assert!(matches!(
        device.ledger.get(foreign, password_binding()).await,
        Err(VaultError::NotFound)
    ));
    assert!(matches!(
        device.ledger.delete(foreign).await,
        Err(VaultError::NotFound)
    ));
    assert!(matches!(
        device
            .ledger
            .put(
                Some(foreign),
                "mine".into(),
                password_binding(),
                Secret::new("x")
            )
            .await,
        Err(VaultError::Format)
    ));
    // Other credentials still work beside it.
    device
        .ledger
        .put(None, "Work".into(), password_binding(), Secret::new("pw"))
        .await
        .unwrap();
    assert_eq!(device.ledger.list().await.unwrap().len(), 1);

    let collection = hub.collections(VAULT_KIND).await.unwrap().remove(0);
    let after = hub.objects(&collection.collection_id);
    let kept = after
        .iter()
        .find(|object| object.object_id == foreign.to_string())
        .unwrap();
    assert_eq!(
        kept,
        before
            .iter()
            .find(|o| o.object_id == foreign.to_string())
            .unwrap()
    );
    assert!(!kept.deleted);
}

#[tokio::test]
async fn a_record_of_a_newer_format_is_hidden_and_never_changed() {
    let hub = Arc::new(MemoryHub::default());
    let (device, _) = device(&hub);
    let foreign: CredentialId = "00000000000000000000000000000043".parse().unwrap();
    let text =
        r#"{"v":2,"label":"x","binding":{"kind":"private_key","public_key":"k"},"secret":"s"}"#;
    write_foreign(&device, &hub, &foreign.to_string(), text).await;
    assert!(device.ledger.list().await.unwrap().is_empty());
    assert!(matches!(
        device.ledger.delete(foreign).await,
        Err(VaultError::NotFound)
    ));
}
