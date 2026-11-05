use super::{race::Device, *};
use crate::account::{
    key_admission::KeyAdmission,
    source::LocalSourceProvider,
    testing::{ACCOUNT_ID, MemoryKeysets, NativeAccounts, PASSWORD},
};
use nocterm_hubs_model::{Account, AccountId};

#[tokio::test]
async fn an_old_displayed_recovery_key_cannot_confirm_a_reissued_key() {
    let device = Device::new();
    let records = MemoryKeysets::default();
    let KeyStatus::Created(first) = device.sign_in(&records, PASSWORD).await else {
        panic!("created")
    };
    let owner = device.owner();
    let keys = owner.keys().unwrap();
    let key = device
        .flow
        .reissue_with(&owner, &records, keys.keyset())
        .await
        .unwrap();
    let second = device.flow.offer(&owner, key).unwrap();
    assert!(matches!(first.confirm().await, Err(HubError::Stale)));
    assert!(
        device
            .flow
            .recovery_key_unconfirmed(&crate::account::testing::hub())
    );
    second.confirm().await.unwrap();
    assert!(
        !device
            .flow
            .recovery_key_unconfirmed(&crate::account::testing::hub())
    );
    assert!(matches!(second.confirm().await, Err(HubError::Stale)));
}

#[tokio::test]
async fn same_account_sign_in_never_revives_an_earlier_recovery_card() {
    let device = Device::new();
    let records = MemoryKeysets::default();
    let KeyStatus::Created(first) = device.sign_in(&records, PASSWORD).await else {
        panic!("created")
    };
    device.sign_in(&records, PASSWORD).await;
    assert!(matches!(first.confirm().await, Err(HubError::Stale)));
    assert!(
        device
            .flow
            .recovery_key_unconfirmed(&crate::account::testing::hub())
    );
}

#[tokio::test]
async fn a_display_account_never_replaces_the_authenticated_native_recipient() {
    let fixture = NativeAccounts::new();
    let provider = LocalSourceProvider::new(fixture.slot().clone(), fixture.subjects().clone());
    let owner = KeyAdmission::scripted(provider.membership().unwrap());
    let account = |id: &str| Account {
        id: AccountId::new(id),
        username: "display".into(),
        display_name: "Display".into(),
        orgs: Vec::new(),
    };
    assert!(crate::directory::checked_account(&owner, account(ACCOUNT_ID)).is_ok());
    assert!(matches!(
        crate::directory::checked_account(&owner, account("account-2")),
        Err(HubError::Stale)
    ));
    fixture.login(1);
    fixture.login(0);
    assert!(matches!(
        crate::directory::checked_account(&owner, account(ACCOUNT_ID)),
        Err(HubError::Stale)
    ));
    assert!(fixture.source().check_current().is_ok());
}
