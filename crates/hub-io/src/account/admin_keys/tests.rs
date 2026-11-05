use std::sync::Arc;

use nocterm_hub_proto::v1;
use nocterm_vault::VaultAccess as _;

use super::*;
use crate::account::Slot;
use crate::account::testing::{Admin, granted, native_operations, signed_in};
use crate::store::memory::MemoryHub;
use crate::store::{Store, VAULT_KIND};

const GROUP: &str = "acme-vault";

/// An account on a device that has confirmed nobody, in "acme", whose administrator granted
/// the key of the organization's vault.
struct Setup {
    hub: Arc<MemoryHub>,
    slot: Arc<Slot>,
    store: Store,
    operations: Arc<crate::account::operations::AccountOperations>,
    _dir: tempfile::TempDir,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = signed_in(dir.path());
    let hub = Arc::new(MemoryHub::default());
    let (_, envelope) = granted(&record, GROUP, Admin::acme());
    hub.add_group(VAULT_KIND, "acme", GROUP, envelope, Admin::acme());
    hub.set_memberships(&[("acme", "Acme", v1::Role::Member)]);
    let operations = native_operations(&slot, hub.clone());
    let store = Store::new(hub.clone(), slot.open().unwrap());
    Setup {
        hub,
        slot,
        store,
        operations,
        _dir: dir,
    }
}

async fn opened_groups(setup: &Setup) -> usize {
    setup.store.layout(VAULT_KIND).await.unwrap().groups.len()
}

fn pinned(slot: &Slot) -> Vec<String> {
    slot.read_cache(|cache| {
        cache
            .map(|cache| {
                cache
                    .pinned()
                    .entries()
                    .map(|(account, key)| format!("{account}:{}", key.fingerprint()))
                    .collect()
            })
            .unwrap_or_default()
    })
}

async fn confirm(keys: &AdminKeys, account: &str, fingerprint: &str) -> Result<(), AdminKeyError> {
    let shown = keys
        .pending()
        .into_iter()
        .find(|offer| offer.key().account_id == account)
        .ok_or(AdminKeyError::NotPending)?;
    shown.confirm(fingerprint).await
}

/// The user compares the fingerprint shown for "acme" with the administrator's and confirms.
async fn confirm_acme(setup: &Setup, keys: &AdminKeys) {
    assert_eq!(opened_groups(setup).await, 0);
    confirm(
        keys,
        Admin::acme().account_id(),
        &Admin::acme().fingerprint(),
    )
    .await
    .unwrap();
    assert_eq!(opened_groups(setup).await, 1);
}

#[tokio::test]
async fn a_group_of_an_unconfirmed_administrator_is_left_out_and_shown_with_its_fingerprint() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());

    assert_eq!(opened_groups(&setup).await, 0);

    assert_eq!(
        keys.pending()
            .into_iter()
            .map(|offer| offer.key().clone())
            .collect::<Vec<_>>(),
        [PendingAdminKey {
            organization: "Acme".into(),
            account_id: Admin::acme().account_id().into(),
            admin: "Ann Admin".into(),
            fingerprint: Admin::acme().fingerprint(),
            replaces: None,
        }]
    );
}

#[tokio::test]
async fn confirming_the_shown_fingerprint_opens_the_group_and_remembers_the_administrator() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());
    assert_eq!(opened_groups(&setup).await, 0);

    confirm(
        &keys,
        Admin::acme().account_id(),
        &Admin::acme().fingerprint(),
    )
    .await
    .unwrap();

    assert_eq!(opened_groups(&setup).await, 1);
    assert!(keys.pending().is_empty());
    assert_eq!(
        pinned(&setup.slot),
        [format!(
            "{}:{}",
            Admin::acme().account_id(),
            Admin::acme().fingerprint()
        )]
    );
}

#[tokio::test]
async fn a_failed_pin_publication_keeps_the_offer_pending_and_the_key_untrusted() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());
    assert_eq!(opened_groups(&setup).await, 0);
    let shown = keys.pending().pop().unwrap();
    let path = std::fs::read_dir(setup._dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "cache")
        })
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(
        shown.confirm(&Admin::acme().fingerprint()).await,
        Err(AdminKeyError::Persistence { .. })
    ));
    assert!(pinned(&setup.slot).is_empty());
    assert_eq!(keys.pending().len(), 1);
    assert_eq!(opened_groups(&setup).await, 0);
}

#[tokio::test]
async fn a_fingerprint_retyped_with_other_grouping_and_case_confirms() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());
    assert_eq!(opened_groups(&setup).await, 0);

    let retyped = Admin::acme()
        .fingerprint()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect::<String>();

    confirm(&keys, Admin::acme().account_id(), &retyped)
        .await
        .unwrap();
}

#[tokio::test]
async fn another_fingerprint_does_not_confirm_and_nothing_is_pinned() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());
    assert_eq!(opened_groups(&setup).await, 0);
    let other = Admin::new("someone-else");

    assert!(matches!(
        confirm(&keys, Admin::acme().account_id(), &other.fingerprint()).await,
        Err(AdminKeyError::Mismatch)
    ));

    assert_eq!(opened_groups(&setup).await, 0);
    assert!(pinned(&setup.slot).is_empty());
    assert_eq!(keys.pending().len(), 1);
}

#[tokio::test]
async fn an_administrator_nobody_waits_for_cannot_be_confirmed() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());

    assert!(matches!(
        confirm(&keys, "stranger", &Admin::acme().fingerprint()).await,
        Err(AdminKeyError::NotPending)
    ));
}

#[tokio::test]
async fn a_hub_that_names_a_granter_with_a_key_of_its_own_gets_no_key_without_confirmation() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());
    let hostile = Admin::new("hostile");
    // The hub offers its own key as the one of the administrator the user already confirmed.
    confirm_acme(&setup, &keys).await;
    setup
        .hub
        .name_granter(GROUP, Admin::acme().account_id(), &hostile);

    assert_eq!(opened_groups(&setup).await, 0);

    let [pending] = keys.pending().try_into().unwrap();
    assert_eq!(pending.key().fingerprint, hostile.fingerprint());
    assert_eq!(pending.key().replaces, Some(Admin::acme().fingerprint()));
}

#[tokio::test]
async fn a_changed_key_replaces_the_pin_only_when_confirmed() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());
    let next = Admin::new(Admin::acme().account_id());
    confirm_acme(&setup, &keys).await;
    setup
        .hub
        .name_granter(GROUP, Admin::acme().account_id(), &next);
    assert_eq!(opened_groups(&setup).await, 0);

    confirm(&keys, next.account_id(), &next.fingerprint())
        .await
        .unwrap();

    assert_eq!(
        pinned(&setup.slot),
        [format!("{}:{}", next.account_id(), next.fingerprint())]
    );
}

#[tokio::test]
async fn a_grant_the_hub_presents_as_the_accounts_own_envelope_does_not_open() {
    let setup = setup();
    setup.hub.name_granter(GROUP, "", Admin::acme());

    assert_eq!(opened_groups(&setup).await, 0);
    assert!(
        AdminKeys::new(setup.operations.clone())
            .pending()
            .is_empty()
    );
}

#[tokio::test]
async fn nothing_is_pending_or_confirmable_while_the_keys_are_closed() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());
    assert_eq!(opened_groups(&setup).await, 0);
    let shown = keys.pending().pop().unwrap();
    setup.slot.lock();
    assert!(keys.pending().is_empty());
    assert!(matches!(
        shown.confirm(&Admin::acme().fingerprint()).await,
        Err(AdminKeyError::Stale)
    ));
}

#[tokio::test]
async fn a_new_pending_administrator_and_a_confirmation_raise_the_keys_notice_once_each() {
    let setup = setup();
    let keys = AdminKeys::new(setup.operations.clone());
    let start = setup.slot.notice().count();

    assert_eq!(opened_groups(&setup).await, 0);
    let found = setup.slot.notice().count();
    assert_eq!(found, start + 1);

    // Looking again finds the same administrator and tells nobody.
    assert_eq!(opened_groups(&setup).await, 0);
    assert_eq!(setup.slot.notice().count(), found);

    confirm(
        &keys,
        Admin::acme().account_id(),
        &Admin::acme().fingerprint(),
    )
    .await
    .unwrap();
    assert_eq!(setup.slot.notice().count(), found + 1);
}

#[tokio::test]
async fn an_administrator_card_cannot_confirm_the_same_grantor_for_a_replacement_recipient() {
    use crate::account::testing::NativeAccounts;
    use crate::store::StoreOps as _;
    let fixture = NativeAccounts::new();
    for index in [0, 1] {
        fixture.login(index);
        let record = fixture.hub(index).keyset().await.unwrap().unwrap();
        let (_, envelope) = granted(&record, GROUP, Admin::acme());
        fixture
            .hub(index)
            .add_group(VAULT_KIND, "acme", GROUP, envelope, Admin::acme());
        fixture
            .hub(index)
            .set_memberships(&[("acme", "Acme", v1::Role::Member)]);
    }
    fixture.login(0);
    let keys = AdminKeys::new(fixture.operations().clone());
    fixture.seed().await;
    fixture.vault().list().await.unwrap();
    let old = keys.pending().pop().unwrap();
    fixture.login(1);
    fixture.seed().await;
    fixture.vault().list().await.unwrap();
    let current = keys.pending().pop().unwrap();
    assert_eq!(
        old.key(),
        current.key(),
        "identical grantor/fingerprint is not recipient authority"
    );
    assert!(matches!(
        old.confirm(&Admin::acme().fingerprint()).await,
        Err(AdminKeyError::Stale)
    ));
    assert_eq!(keys.pending().len(), 1);
    current.confirm(&Admin::acme().fingerprint()).await.unwrap();
    assert!(keys.pending().is_empty());
    fixture.login(0);
    fixture.vault().list().await.unwrap();
    assert!(matches!(
        old.confirm(&Admin::acme().fingerprint()).await,
        Err(AdminKeyError::Stale)
    ));
    assert_eq!(keys.pending().len(), 1);
}
