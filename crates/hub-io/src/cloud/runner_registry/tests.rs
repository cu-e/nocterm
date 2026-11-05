//! Actual native keysets and shared Store fixtures; no new protocol fake.
use super::*;
use crate::{
    account::{Slot, operations::AccountOperations, testing},
    store::memory::{MemoryHub, StoreBoundary},
};
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    identity::*,
    journal::{PublicationAllocation, PublicationBudget, PublicationWorkBudget},
};
use std::sync::Arc;
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
fn scope(bound: &BoundOperations) -> AccountScope {
    let key = bound
        .source()
        .native()
        .with_current(|keys| {
            Ok(Fingerprint::from_bytes(
                Sha256::digest(keys.keyset().public_key().to_bytes()).into(),
            ))
        })
        .unwrap();
    AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("trusted-registry-hub").unwrap(),
            Fingerprint::from_bytes([3; 32]),
        ),
        AccountIdentity::new(NamespaceId::new(testing::ACCOUNT_ID).unwrap(), key),
    )
}
fn registration(scope: &AccountScope, id: u8, epoch: u64, key: u8) -> RunnerRegistration {
    RunnerRegistration::new(
        JournalCatalogBinding::new(
            scope.clone(),
            RunnerId::from_bytes([id; 16]),
            RunnerKeyEpoch::new(epoch).unwrap(),
        ),
        ChatMaterialPins {
            encryption: [key; 32],
            signing: [key + 1; 32],
            proof: [key + 2; 32],
        },
        "Trusted Rig public keys stay encrypted".to_owned(),
    )
    .unwrap()
}
async fn load(
    bound: &BoundOperations,
    scope: &AccountScope,
    registration: Option<RunnerRegistration>,
) -> Result<Allocated<(u64, Vec<RunnerRegistration>)>, RunnerRegistryError> {
    let allocation = reserve(runner_registry_work_bytes()?);
    let work = Allocated::new(
        bound
            .capture_bounded(MAX_RESPONSE, allocation.clone())
            .map_err(account)?,
        allocation,
    );
    let result = storage::load_or_register(work.value(), scope, registration.as_ref())
        .await
        .map_err(|error| retained(error, work.allocation().clone()))?;
    bound
        .source()
        .publish(|| Allocated::new(result, reserve(RUNNER_REGISTRY_RESIDENT_BYTES)))
        .map_err(account)
}
#[tokio::test]
async fn account_runner_registry_restores_on_normal_second_unlock_and_reuses_lost_cas_reply() {
    let hub = Arc::new(MemoryHub::default());
    let a_dir = tempfile::tempdir().unwrap();
    let (_a_accounts, a_slot, record) = testing::signed_in(a_dir.path());
    let (_a_operations, a) = bound(&a_slot, hub.clone());
    let scope = scope(&a);
    let original = registration(&scope, 4, 1, 6);
    let (entered, release) = hub.pause_next_error(StoreBoundary::PutReady, HubError::NotSignedIn);
    let (pending_bound, pending_scope) = (a.clone(), scope.clone());
    let pending =
        tokio::spawn(async move { load(&pending_bound, &pending_scope, Some(original)).await });
    entered.wait().await;
    release.release();
    let created = pending.await.unwrap().unwrap();
    assert_eq!(created.value().0, 1);
    let collection = records::collection_id(&scope).unwrap();
    let stored = hub.objects(&collection);
    assert_eq!(stored.len(), 1);
    assert!(
        !stored[0]
            .ciphertext
            .windows(16)
            .any(|slice| slice == b"Trusted Rig publ")
    );
    let b_dir = tempfile::tempdir().unwrap();
    let (_b_accounts, b_slot) = testing::second_device(b_dir.path(), &record);
    let (_b_operations, b) = bound(&b_slot, hub.clone());
    a_slot.lock();
    assert!(load(&a, &scope, None).await.is_err());
    let restored = load(&b, &scope, None).await.unwrap();
    assert_eq!(restored.value().1, created.value().1);
    assert_eq!(
        load(&b, &scope, Some(registration(&scope, 4, 1, 6)))
            .await
            .unwrap()
            .value()
            .0,
        1
    );
    assert_eq!(hub.objects(&collection)[0].ciphertext, stored[0].ciphertext);
    for (epoch, pin) in [(2, 6), (1, 20)] {
        assert!(matches!(
            load(&b, &scope, Some(registration(&scope, 4, epoch, pin))).await,
            Err(RunnerRegistryError::Rotation)
        ));
    }
    assert_eq!(hub.objects(&collection)[0].version, 1);
}
#[tokio::test]
async fn account_runner_registry_cas_race_keeps_other_runner_and_honors_bounds_and_tombstone() {
    let hub = Arc::new(MemoryHub::default());
    let directory = tempfile::tempdir().unwrap();
    let (_accounts, slot, _) = testing::signed_in(directory.path());
    let (_operations, source) = bound(&slot, hub.clone());
    let scope = scope(&source);
    load(&source, &scope, Some(registration(&scope, 1, 1, 6)))
        .await
        .unwrap();
    let (entered, release) = hub.pause_next(StoreBoundary::Put);
    let (old_source, old_scope) = (source.clone(), scope.clone());
    let pending = tokio::spawn(async move {
        load(
            &old_source,
            &old_scope,
            Some(registration(&old_scope, 2, 1, 6)),
        )
        .await
    });
    entered.wait().await;
    load(&source, &scope, Some(registration(&scope, 3, 1, 6)))
        .await
        .unwrap();
    release.release();
    assert!(matches!(
        pending.await.unwrap(),
        Err(RunnerRegistryError::Conflict)
    ));
    let result = load(&source, &scope, None).await.unwrap();
    assert_eq!(
        result
            .value()
            .1
            .iter()
            .map(|row| *row.binding().runner().as_bytes())
            .collect::<Vec<_>>(),
        [[1; 16], [3; 16]]
    );
    for id in 4..=17 {
        load(&source, &scope, Some(registration(&scope, id, 1, 6)))
            .await
            .unwrap();
    }
    assert!(matches!(
        load(&source, &scope, Some(registration(&scope, 18, 1, 6))).await,
        Err(RunnerRegistryError::Capacity)
    ));
    let collection = records::collection_id(&scope).unwrap();
    let ops = source.capture().unwrap();
    let deleted = nocterm_hub_proto::v1::DeleteObjectRequest {
        collection_id: collection.clone(),
        object_id: records::OBJECT.into(),
        expected_version: 16,
    };
    ops.ops().delete_object(deleted).await.unwrap();
    assert!(matches!(
        load(&source, &scope, None).await,
        Err(RunnerRegistryError::Deleted)
    ));
    assert!(matches!(
        load(&source, &scope, Some(registration(&scope, 18, 1, 6))).await,
        Err(RunnerRegistryError::Deleted)
    ));
    assert_eq!(hub.objects(&collection)[0].version, 17);
}
#[tokio::test]
async fn account_runner_registry_rejects_late_original_login_publication_and_context_tampering() {
    let hub = Arc::new(MemoryHub::default());
    let directory = tempfile::tempdir().unwrap();
    let (_accounts, slot, record) = testing::signed_in(directory.path());
    let (operations, subjects) = testing::native_operations_with_subjects(&slot, hub.clone());
    let captured = operations
        .bind(operations.current_opened_source().unwrap())
        .unwrap();
    let scope = scope(&captured);
    load(&captured, &scope, Some(registration(&scope, 1, 1, 6)))
        .await
        .unwrap();
    let (entered, release) = hub.pause_next(StoreBoundary::ObjectReady);
    let (old, expected) = (captured.clone(), scope.clone());
    let pending = tokio::spawn(async move { load(&old, &expected, None).await });
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
    assert!(load(&captured, &scope, None).await.is_err());
    assert_eq!(load(&new, &scope, None).await.unwrap().value().0, 1);
    let foreign = AccountScope::new(
        scope.hub().clone(),
        AccountIdentity::new(
            scope.account().id().clone(),
            Fingerprint::from_bytes([99; 32]),
        ),
    );
    assert!(matches!(
        load(&new, &foreign, None).await,
        Err(RunnerRegistryError::Binding)
    ));
    let row = registration(&scope, 1, 1, 6);
    assert!(RunnerRegistration::new(row.binding().clone(), row.pins(), "x".repeat(129)).is_err());
    let bytes = records::encode(&scope, 1, &[row]).unwrap();
    let mut tampered = bytes.to_vec();
    tampered[0] ^= 1;
    assert!(records::decode(&scope, 1, &tampered).is_err());
    assert!(records::decode(&scope, 2, &bytes).is_err());
    assert!(records::decode(&scope, 1, &vec![0; records::MAX_PLAINTEXT + 1]).is_err());
}
