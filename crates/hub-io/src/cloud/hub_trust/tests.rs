//! Normal native account keys and the shared Store conformance fake.
use super::*;
use crate::{
    account::{Slot, operations::AccountOperations, testing},
    store::memory::{MemoryHub, StoreBoundary},
};
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    journal::{PublicationAllocation, PublicationBudget, PublicationWorkBudget},
};

fn reserve(bytes: usize) -> AllocationLease {
    PublicationBudget::maintenance(IntakePoolLimits::new(bytes, 1, 1).unwrap())
        .try_reserve(PublicationAllocation::new(bytes, 1).unwrap())
        .unwrap()
}
fn bound(slot: &Arc<Slot>, hub: Arc<MemoryHub>) -> (Arc<AccountOperations>, BoundOperations) {
    let operations = testing::native_operations(slot, hub);
    let bound = operations
        .bind(operations.current_opened_source().unwrap())
        .unwrap();
    (operations, bound)
}
async fn load(
    bound: &BoundOperations,
    authority: Option<[u8; 32]>,
) -> Result<Allocated<[u8; 32]>, CloudTrustError> {
    let work = reserve(cloud_trust_work_bytes()?);
    let opened = Allocated::new(
        bound
            .capture_bounded(MAX_RESPONSE, work.clone())
            .map_err(account)?,
        work.clone(),
    );
    let normal = records::NormalAccount::capture(bound, testing::HUB_UID)?;
    let pin = storage::load_or_register(opened.value(), &normal.context(), authority)
        .await
        .map_err(|error| retained(error, work))?;
    bound
        .source()
        .publish(|| Allocated::new(pin, reserve(CLOUD_TRUST_RESIDENT_BYTES)))
        .map_err(account)
}

#[tokio::test]
async fn account_cloud_trust_restores_on_normal_second_unlock_and_reuses_lost_reply() {
    let hub = Arc::new(MemoryHub::default());
    let a_dir = tempfile::tempdir().unwrap();
    let (_a_accounts, a_slot, record) = testing::signed_in(a_dir.path());
    let (_a_operations, a) = bound(&a_slot, hub.clone());
    assert!(matches!(
        load(&a, None).await,
        Err(CloudTrustError::NotConfigured)
    ));
    let context = records::NormalAccount::capture(&a, testing::HUB_UID)
        .unwrap()
        .context();
    let collection = records::collection_id(&context);
    assert!(hub.objects(&collection).is_empty());
    let (entered, release) = hub.pause_next_error(StoreBoundary::PutReady, HubError::NotSignedIn);
    let old = a.clone();
    let pending = tokio::spawn(async move { load(&old, Some([23; 32])).await });
    entered.wait().await;
    release.release();
    assert_eq!(pending.await.unwrap().unwrap().value(), &[23; 32]);
    let stored = hub.objects(&collection);
    assert_eq!(stored.len(), 1);
    assert!(
        !stored[0]
            .ciphertext
            .windows(32)
            .any(|window| window == [23; 32])
    );
    let b_dir = tempfile::tempdir().unwrap();
    let (_b_accounts, b_slot) = testing::second_device(b_dir.path(), &record);
    let (_b_operations, b) = bound(&b_slot, hub.clone());
    a_slot.lock();
    assert!(load(&a, None).await.is_err());
    assert_eq!(load(&b, None).await.unwrap().value(), &[23; 32]);
    assert_eq!(load(&b, Some([23; 32])).await.unwrap().value(), &[23; 32]);
    assert!(matches!(
        load(&b, Some([24; 32])).await,
        Err(CloudTrustError::RotationRequired)
    ));
    assert_eq!(hub.objects(&collection)[0].ciphertext, stored[0].ciphertext);
    assert_eq!(hub.objects(&collection)[0].version, 1);
}

#[tokio::test]
async fn account_cloud_trust_refuses_late_relogin_context_tamper_and_deleted_pin() {
    let hub = Arc::new(MemoryHub::default());
    let directory = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = testing::signed_in(directory.path());
    let (operations, subjects) = testing::native_operations_with_subjects(&slot, hub.clone());
    let captured = operations
        .bind(operations.current_opened_source().unwrap())
        .unwrap();
    load(&captured, Some([31; 32])).await.unwrap();
    let (entered, release) = hub.pause_next(StoreBoundary::ObjectReady);
    let old = captured.clone();
    let pending = tokio::spawn(async move { load(&old, None).await });
    entered.wait().await;
    slot.lock();
    subjects.sign_in(testing::hub(), "another-account", "device-b");
    subjects.sign_in(testing::hub(), testing::ACCOUNT_ID, "device");
    slot.install(
        testing::open(&record, &testing::unlock(testing::PASSWORD)),
        testing::cache(&record, crate::cache::UnlockMethod::Passphrase),
    );
    let new = operations
        .bind(operations.current_opened_source().unwrap())
        .unwrap();
    release.release();
    assert!(pending.await.unwrap().is_err());
    assert!(load(&captured, None).await.is_err());
    assert_eq!(load(&new, None).await.unwrap().value(), &[31; 32]);
    let context = records::NormalAccount::capture(&new, testing::HUB_UID)
        .unwrap()
        .context();
    let encoded = records::encode(&context, [31; 32]).unwrap();
    let mut moved = context.clone();
    moved[0] ^= 1;
    assert!(records::decode(&moved, &encoded).is_err());
    assert!(records::decode(&context, &[0; records::MAX_PLAINTEXT + 1]).is_err());
    let collection = records::collection_id(&context);
    let ciphertext = hub.objects(&collection)[0].ciphertext.clone();
    let mut tampered = ciphertext.clone();
    tampered[0] ^= 1;
    hub.tamper(&collection, records::OBJECT, tampered);
    assert!(load(&new, None).await.is_err());
    hub.tamper(&collection, records::OBJECT, ciphertext);
    let work = new.capture().unwrap();
    work.ops()
        .delete_object(nocterm_hub_proto::v1::DeleteObjectRequest {
            collection_id: collection,
            object_id: records::OBJECT.into(),
            expected_version: 1,
        })
        .await
        .unwrap();
    assert!(matches!(
        load(&new, None).await,
        Err(CloudTrustError::Deleted)
    ));
    assert!(matches!(
        load(&new, Some([31; 32])).await,
        Err(CloudTrustError::Deleted)
    ));
}

#[test]
fn account_cloud_trust_import_checks_original_namespace_before_storage() {
    use nocterm_cloud_model::{
        identity::{RunnerId, RunnerKeyEpoch},
        journal::JournalCatalogBinding,
    };
    let hub = Arc::new(MemoryHub::default());
    let directory = tempfile::tempdir().unwrap();
    let (_accounts, slot, _) = testing::signed_in(directory.path());
    let (_operations, bound) = bound(&slot, hub.clone());
    let normal = records::NormalAccount::capture(&bound, testing::HUB_UID).unwrap();
    let scope = normal.scope([15; 32]).unwrap();
    let binding = JournalCatalogBinding::new(
        scope.clone(),
        RunnerId::from_bytes([7; 16]),
        RunnerKeyEpoch::new(1).unwrap(),
    );
    let identity = super::super::CloudRunnerIdentity::create_new(
        &directory.path().join("identity"),
        binding.clone(),
    )
    .unwrap();
    let public = identity.public_identity().unwrap();
    let runner = || {
        RunnerRegistration::new(
            binding.clone(),
            super::super::ChatMaterialPins {
                encryption: *public.encryption_key(),
                signing: *public.signing_key(),
                proof: *public.proof_key(),
            },
            "Independently compared runner".into(),
        )
        .unwrap()
    };
    let setup =
        CloudRunnerSetup::new(testing::hub(), testing::HUB_UID.into(), [15; 32], runner()).unwrap();
    normal.require_setup(&setup).unwrap();
    let moved = CloudRunnerSetup::new(
        nocterm_hubs_model::HubAddress::parse("other-hub.test")
            .unwrap()
            .into(),
        testing::HUB_UID.into(),
        [15; 32],
        runner(),
    )
    .unwrap();
    assert!(matches!(
        normal.require_setup(&moved),
        Err(CloudTrustError::Binding)
    ));
    for setup in [
        CloudRunnerSetup::new(testing::hub(), "moved-hub".into(), [15; 32], runner()),
        CloudRunnerSetup::new(testing::hub(), testing::HUB_UID.into(), [16; 32], runner()),
    ] {
        assert!(matches!(setup, Err(CloudTrustError::Binding)));
    }
    assert_ne!(scope.hub().instance().as_str(), testing::HUB_UID);
    assert!(
        hub.objects(&records::collection_id(&normal.context()))
            .is_empty()
    );
}
