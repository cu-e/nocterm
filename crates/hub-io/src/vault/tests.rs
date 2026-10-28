use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nocterm_session::Target;

use super::*;
use crate::account::testing::{PASSWORD, signed_in};
use crate::store::VAULT_KIND;
use crate::store::memory::MemoryHub;

fn vault() -> (HubVault, Arc<MemoryHub>, tempfile::TempDir) {
    let hub = Arc::new(MemoryHub::default());
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = signed_in(dir.path());
    hub.set_keyset(Some(record));
    (
        HubVault::new(Handle::current(), slot, hub.clone()),
        hub,
        dir,
    )
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

const ACME_VAULT: &str = "acme-vault";

/// A vault whose account belongs to the organization "acme" with `role` and holds the key of
/// its vault collection; the key is what the other members seal with.
fn vault_in_acme(
    role: nocterm_hub_proto::v1::Role,
) -> (
    HubVault,
    Arc<MemoryHub>,
    nocterm_keys::CollectionKey,
    tempfile::TempDir,
) {
    let hub = Arc::new(MemoryHub::default());
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = signed_in(dir.path());
    hub.set_keyset(Some(record.clone()));
    let (key, envelope) = crate::account::testing::granted(&record, ACME_VAULT);
    hub.add_group(VAULT_KIND, "acme", ACME_VAULT, envelope);
    hub.set_memberships(&[("acme", "Acme", role)]);
    (
        HubVault::new(Handle::current(), slot, hub.clone()),
        hub,
        key,
        dir,
    )
}

/// Another member stores a credential in the group's vault.
fn member_saves(
    hub: &MemoryHub,
    key: &nocterm_keys::CollectionKey,
    id: CredentialId,
    label: &str,
    secret: &str,
) {
    let plain = codec::encode(label, &binding(), &Secret::new(secret)).unwrap();
    let object_id = id.to_string();
    let sealed = key
        .seal_object(ACME_VAULT, &object_id, codec::KIND, &plain)
        .unwrap();
    hub.write_as_member(ACME_VAULT, &object_id, codec::KIND, sealed);
}

fn shared_id() -> CredentialId {
    CredentialId::from_bytes([9; 16])
}

#[tokio::test(flavor = "multi_thread")]
async fn a_groups_credentials_are_listed_with_their_group_and_the_members_access() {
    use nocterm_hub_proto::v1::Role;
    for (role, writable) in [(Role::Guest, false), (Role::Member, true)] {
        let (vault, hub, key, _dir) = vault_in_acme(role);
        member_saves(&hub, &key, shared_id(), "Team", "team-pw");
        let own = vault
            .put(None, "Own".into(), binding(), Secret::new("own-pw"))
            .await
            .unwrap();
        let listed = vault.list().await.unwrap();
        let group = |label: &str| {
            listed
                .iter()
                .find(|info| info.label == label)
                .map(|info| info.group.clone())
        };
        assert_eq!(group("Own"), Some(None));
        assert_eq!(
            group("Team"),
            Some(Some(nocterm_vault::CredentialGroup {
                name: "Acme".into(),
                writable
            }))
        );
        assert_eq!(vault.get(own, binding()).await.unwrap().expose(), "own-pw");
        assert_eq!(
            vault.get(shared_id(), binding()).await.unwrap().expose(),
            "team-pw"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_groups_credential_only_answers_for_its_own_binding() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Member);
    member_saves(&hub, &key, shared_id(), "Team", "team-pw");
    let other = CredentialBinding::KeyPassphrase {
        path: PathBuf::from("/k"),
    };
    assert!(matches!(
        vault.get(shared_id(), other).await,
        Err(VaultError::Binding)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_changes_and_deletes_a_group_credential_in_the_group_only() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Member);
    member_saves(&hub, &key, shared_id(), "Team", "team-pw");
    vault
        .put(
            Some(shared_id()),
            "Team".into(),
            binding(),
            Secret::new("new-pw"),
        )
        .await
        .unwrap();
    let stored = hub.objects(ACME_VAULT);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].version, 2);
    let opened = key
        .open_object(
            ACME_VAULT,
            &stored[0].object_id,
            codec::KIND,
            &stored[0].ciphertext,
        )
        .unwrap();
    assert!(String::from_utf8_lossy(&opened).contains("new-pw"));
    let personal = hub.collections(VAULT_KIND).await.unwrap();
    assert!(
        personal
            .iter()
            .filter(|c| c.personal)
            .all(|c| hub.objects(&c.collection_id).is_empty()),
        "nothing reached the personal vault"
    );
    assert_eq!(
        vault.get(shared_id(), binding()).await.unwrap().expose(),
        "new-pw"
    );
    vault.delete(shared_id()).await.unwrap();
    assert!(hub.objects(ACME_VAULT)[0].deleted);
    assert!(vault.list().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_credential_goes_to_the_personal_vault_even_for_a_group_member() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Owner);
    member_saves(&hub, &key, shared_id(), "Team", "team-pw");
    let id = vault
        .put(None, "Own".into(), binding(), Secret::new("own-pw"))
        .await
        .unwrap();
    assert_eq!(hub.objects(ACME_VAULT).len(), 1);
    assert_eq!(
        hub.objects(ACME_VAULT)[0].object_id,
        shared_id().to_string()
    );
    let listed = vault.list().await.unwrap();
    let own = listed.iter().find(|info| info.id == id).unwrap();
    assert!(own.group.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guest_reads_a_group_credential_but_cannot_change_or_delete_it() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Guest);
    member_saves(&hub, &key, shared_id(), "Team", "team-pw");
    let written = vault
        .put(
            Some(shared_id()),
            "Team".into(),
            binding(),
            Secret::new("mine"),
        )
        .await;
    assert!(matches!(written, Err(VaultError::ReadOnly)));
    assert!(matches!(
        vault.delete(shared_id()).await,
        Err(VaultError::ReadOnly)
    ));
    assert_eq!(hub.objects(ACME_VAULT)[0].version, 1);
    assert_eq!(
        vault.get(shared_id(), binding()).await.unwrap().expose(),
        "team-pw"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_by_the_hub_of_a_group_write_is_read_only_too() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Member);
    member_saves(&hub, &key, shared_id(), "Team", "team-pw");
    hub.refuse_writes_to(ACME_VAULT);
    let written = vault
        .put(
            Some(shared_id()),
            "Team".into(),
            binding(),
            Secret::new("mine"),
        )
        .await;
    assert!(matches!(written, Err(VaultError::ReadOnly)));
    assert!(!hub.objects(ACME_VAULT)[0].deleted);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_record_this_client_cannot_read_is_never_overwritten_or_deleted() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Member);
    let id = shared_id();
    let object_id = id.to_string();
    let sealed = key
        .seal_object(ACME_VAULT, &object_id, codec::KIND, br#"{"v":2}"#)
        .unwrap();
    hub.write_as_member(ACME_VAULT, &object_id, codec::KIND, sealed);
    let written = vault
        .put(Some(id), "Team".into(), binding(), Secret::new("mine"))
        .await;
    assert!(matches!(written, Err(VaultError::Format)));
    assert!(matches!(vault.delete(id).await, Err(VaultError::NotFound)));
    assert_eq!(hub.objects(ACME_VAULT)[0].version, 1);
    assert!(vault.list().await.unwrap().is_empty());
}

fn in_acme() -> Space {
    Space::Group("acme".to_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_personal_credential_moved_into_a_group_is_sealed_with_the_groups_key_alone() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Member);
    let id = vault
        .put(None, "Own".into(), binding(), Secret::new("own-pw"))
        .await
        .unwrap();
    let moved = vault
        .ledger()
        .carry(&Space::Personal, &in_acme(), &[id])
        .await
        .unwrap();
    assert_eq!(moved, [id]);
    let stored = hub.objects(ACME_VAULT);
    assert_eq!(stored.len(), 1);
    let opened = key
        .open_object(
            ACME_VAULT,
            &stored[0].object_id,
            codec::KIND,
            &stored[0].ciphertext,
        )
        .unwrap();
    assert!(String::from_utf8_lossy(&opened).contains("own-pw"));
    let listed = vault.list().await.unwrap();
    assert_eq!(listed.len(), 1, "no personal copy is left");
    assert!(listed[0].group.is_some());
    assert_eq!(vault.get(id, binding()).await.unwrap().expose(), "own-pw");
    vault
        .put(Some(id), "Own".into(), binding(), Secret::new("new-pw"))
        .await
        .unwrap();
    assert_eq!(
        hub.objects(ACME_VAULT)[0].version,
        3,
        "later writes stay in the group"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_credential_moved_out_becomes_personal_and_leaves_the_group() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Member);
    member_saves(&hub, &key, shared_id(), "Team", "team-pw");
    let moved = vault
        .ledger()
        .carry(&in_acme(), &Space::Personal, &[shared_id()])
        .await
        .unwrap();
    assert_eq!(moved, [shared_id()]);
    assert!(hub.objects(ACME_VAULT)[0].deleted);
    let listed = vault.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].group.is_none());
    assert_eq!(
        vault.get(shared_id(), binding()).await.unwrap().expose(),
        "team-pw"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_guest_cannot_move_a_credential_into_the_group_and_nothing_is_copied() {
    let (vault, hub, _key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Guest);
    let id = vault
        .put(None, "Own".into(), binding(), Secret::new("own-pw"))
        .await
        .unwrap();
    let moved = vault
        .ledger()
        .carry(&Space::Personal, &in_acme(), &[id])
        .await;
    assert!(matches!(moved, Err(VaultError::ReadOnly)));
    assert!(hub.objects(ACME_VAULT).is_empty());
    let listed = vault.list().await.unwrap();
    assert!(listed.len() == 1 && listed[0].group.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_credential_the_source_does_not_hold_stays_where_it_is() {
    let (vault, hub, key, _dir) = vault_in_acme(nocterm_hub_proto::v1::Role::Member);
    member_saves(&hub, &key, shared_id(), "Team", "team-pw");
    let moved = vault
        .ledger()
        .carry(&Space::Personal, &Space::Personal, &[shared_id()])
        .await
        .unwrap();
    assert!(moved.is_empty());
    assert!(!hub.objects(ACME_VAULT)[0].deleted);
}
