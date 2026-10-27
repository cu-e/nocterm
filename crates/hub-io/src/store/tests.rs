use std::sync::Arc;

use super::memory::MemoryHub;
use super::*;
use crate::account::testing::open_keys;

fn store(hub: &Arc<MemoryHub>) -> Store {
    Store::new(hub.clone(), open_keys())
}

#[tokio::test]
async fn the_collection_of_a_kind_is_created_once_and_found_again() {
    let hub = Arc::new(MemoryHub::default());
    let store = store(&hub);
    assert!(store.find(VAULT_KIND).await.unwrap().is_none());
    let first = store.ensure(VAULT_KIND).await.unwrap();
    let again = store.ensure(VAULT_KIND).await.unwrap();
    assert_eq!(first.id(), again.id());
}

#[tokio::test]
async fn objects_are_stored_sealed_and_a_stale_write_conflicts() {
    let hub = Arc::new(MemoryHub::default());
    let store = store(&hub);
    let at = store.ensure(VAULT_KIND).await.unwrap();
    let v1 = store
        .put(&at, "credential", "o1", b"plain text", 0)
        .await
        .unwrap();
    assert_eq!(v1.version, 1);
    let stored = hub.objects(at.id());
    assert_eq!(stored.len(), 1);
    assert!(!String::from_utf8_lossy(&stored[0].ciphertext).contains("plain text"));
    assert!(matches!(
        store.put(&at, "credential", "o1", b"again", 0).await,
        Err(OpError::Conflict)
    ));
    let v2 = store
        .put(&at, "credential", "o1", b"again", 1)
        .await
        .unwrap();
    assert_eq!(v2.version, 2);
    let opened = at
        .key()
        .open_object(
            at.id(),
            "o1",
            "credential",
            &hub.objects(at.id())[0].ciphertext,
        )
        .unwrap();
    assert_eq!(opened.as_slice(), b"again");
}

#[tokio::test]
async fn removing_leaves_a_tombstone_in_the_change_feed() {
    let hub = Arc::new(MemoryHub::default());
    let store = store(&hub);
    let at = store.ensure(PROFILES_KIND).await.unwrap();
    store.put(&at, "profile", "p", b"{}", 0).await.unwrap();
    store.remove(&at, "p", 1).await.unwrap();
    let batch = store.changes(&at, 1).await.unwrap();
    assert_eq!(batch.changes.len(), 1);
    assert!(batch.changes[0].deleted);
    assert_eq!(batch.head, 2);
}

#[tokio::test]
async fn other_kinds_and_organization_collections_are_never_picked() {
    let hub = Arc::new(MemoryHub::default());
    hub.add_foreign("future-kind", "");
    hub.add_foreign(VAULT_KIND, "org-1");
    let store = store(&hub);
    assert!(store.find(VAULT_KIND).await.unwrap().is_none());
    let mine = store.ensure(VAULT_KIND).await.unwrap();
    assert!(!mine.id().starts_with("foreign"));
}

#[tokio::test]
async fn when_two_devices_each_made_a_collection_the_oldest_is_used() {
    let hub = Arc::new(MemoryHub::default());
    let one = store(&hub);
    let first = one.ensure(VAULT_KIND).await.unwrap();
    // A second device that listed nothing before the first one's create made its own.
    let keys = open_keys();
    let (key, envelope) = keys.new_collection("later").unwrap();
    hub.create_collection(v1::CreateCollectionRequest {
        org_id: String::new(),
        kind: VAULT_KIND.into(),
        sealed_name: key.seal_name("later", VAULT_KIND).unwrap(),
        wrapped_key: envelope,
        collection_id: "later".into(),
    })
    .await
    .unwrap();
    assert_eq!(
        one.find(VAULT_KIND).await.unwrap().unwrap().id(),
        first.id()
    );
}

#[tokio::test]
async fn a_key_envelope_the_hub_altered_is_damage_not_a_new_collection() {
    let hub = Arc::new(MemoryHub::default());
    let store = store(&hub);
    let at = store.ensure(VAULT_KIND).await.unwrap();
    // Another client session has no cached key: it must open the envelope the hub returns.
    let fresh = Store::new(hub.clone(), open_keys());
    hub.tamper_key(at.id(), vec![0; 64]);
    assert!(matches!(
        fresh.find(VAULT_KIND).await,
        Err(OpError::Damaged(_))
    ));
}

#[tokio::test]
async fn an_unreachable_hub_is_a_hub_error_and_creates_nothing() {
    let hub = Arc::new(MemoryHub::default());
    hub.set_offline(true);
    let store = store(&hub);
    assert!(matches!(
        store.ensure(VAULT_KIND).await,
        Err(OpError::Hub(HubError::Unreachable { .. }))
    ));
    hub.set_offline(false);
    assert!(store.find(VAULT_KIND).await.unwrap().is_none());
}

fn member_of_acme(role: v1::Role) -> (Arc<MemoryHub>, Store, v1::AccountKeyset) {
    let hub = Arc::new(MemoryHub::default());
    let (keys, record) = crate::account::testing::open_keys_with_record();
    hub.set_memberships(&[("acme", "Acme", role)]);
    (hub.clone(), Store::new(hub, keys), record)
}

#[tokio::test]
async fn a_group_collection_with_a_granted_key_is_used_with_the_members_role() {
    for (role, writable) in [
        (v1::Role::Guest, false),
        (v1::Role::Member, true),
        (v1::Role::Admin, true),
        (v1::Role::Owner, true),
    ] {
        let (hub, store, record) = member_of_acme(role);
        let (key, envelope) = crate::account::testing::granted(&record, "acme-vault");
        hub.add_group(VAULT_KIND, "acme", "acme-vault", envelope);
        let layout = store.layout(VAULT_KIND).await.unwrap();
        assert!(layout.personal.is_none());
        let [group] = layout.groups.as_slice() else {
            panic!("one group expected");
        };
        assert_eq!(
            (group.org_id.as_str(), group.name.as_str(), group.writable),
            ("acme", "Acme", writable)
        );
        let sealed = key
            .seal_object("acme-vault", "o", "credential", b"x")
            .unwrap();
        let opened = group
            .at
            .key()
            .open_object("acme-vault", "o", "credential", &sealed)
            .unwrap();
        assert_eq!(opened.as_slice(), b"x");
    }
}

#[tokio::test]
async fn a_group_without_a_key_for_the_account_is_left_out_not_an_error() {
    let (hub, store, record) = member_of_acme(v1::Role::Member);
    hub.add_foreign(VAULT_KIND, "acme");
    let layout = store.layout(VAULT_KIND).await.unwrap();
    assert!(layout.groups.is_empty());
    assert!(store.shared(VAULT_KIND, "acme").await.unwrap().is_none());
    let (_, envelope) = crate::account::testing::granted(&record, "acme-vault");
    hub.add_group(VAULT_KIND, "acme", "acme-vault", envelope);
    hub.tamper_key("acme-vault", vec![1; 60]);
    assert!(store.layout(VAULT_KIND).await.unwrap().groups.is_empty());
}

#[tokio::test]
async fn an_organization_the_account_does_not_belong_to_is_never_used() {
    let (hub, store, record) = member_of_acme(v1::Role::Member);
    let (_, envelope) = crate::account::testing::granted(&record, "other-vault");
    hub.add_group(VAULT_KIND, "other", "other-vault", envelope);
    assert!(store.layout(VAULT_KIND).await.unwrap().groups.is_empty());
}

#[tokio::test]
async fn the_account_never_creates_a_group_collection_and_ensure_stays_personal() {
    let (hub, store, _) = member_of_acme(v1::Role::Owner);
    let own = store.ensure(VAULT_KIND).await.unwrap();
    assert!(
        hub.collections(VAULT_KIND)
            .await
            .unwrap()
            .iter()
            .all(|c| c.org_id.is_empty())
    );
    let layout = store.layout(VAULT_KIND).await.unwrap();
    assert_eq!(
        layout.personal.map(|at| at.id().to_owned()),
        Some(own.id().to_owned())
    );
    assert!(layout.groups.is_empty());
}
