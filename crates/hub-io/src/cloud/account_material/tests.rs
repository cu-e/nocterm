//! Real account-keyset and purpose crypto over the shared native memory Store.
use super::*;
use crate::{
    account::{
        Accounts, OpenKeys, Slot,
        operations::{AccountOperations, BoundOperations},
        testing,
    },
    cache::UnlockMethod,
    store::{
        StoreOps,
        memory::{MemoryHub, StoreBoundary},
    },
};
use nocterm_cloud_model::{
    authority::StreamBinding,
    coordinator::IntakePoolLimits,
    identity::*,
    journal::{PublicationAllocation, PublicationBudget, PublicationWorkBudget},
};
use nocterm_hub_client::Keyset;
use nocterm_hub_model::cloud::{JournalClass, JournalCoordinate};
use nocterm_hub_proto::v1;
use nocterm_keys::PinnedKeys;
use sha2::{Digest, Sha256};

struct Device {
    _dir: tempfile::TempDir,
    _accounts: Accounts,
    slot: Arc<Slot>,
    operations: Arc<AccountOperations>,
    hub: Arc<MemoryHub>,
    record: v1::AccountKeyset,
}
impl Device {
    fn open(hub: Arc<MemoryHub>, record: v1::AccountKeyset) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let accounts = Accounts::new(dir.path().to_path_buf());
        let slot = accounts.slot(&testing::hub()).unwrap();
        let keys = Keyset::open(
            &record,
            testing::ACCOUNT_ID,
            &testing::unlock(testing::PASSWORD),
        )
        .unwrap();
        slot.install(
            OpenKeys::pinning(keys, PinnedKeys::new()),
            testing::cache(&record, UnlockMethod::Passphrase),
        );
        let operations = testing::native_operations(&slot, hub.clone());
        Self {
            _dir: dir,
            _accounts: accounts,
            slot,
            operations,
            hub,
            record,
        }
    }
    fn bound(&self) -> BoundOperations {
        self.operations
            .bind(self.operations.current_opened_source().unwrap())
            .unwrap()
    }
    fn login(&mut self) {
        let keys = Keyset::open(
            &self.record,
            testing::ACCOUNT_ID,
            &testing::unlock(testing::PASSWORD),
        )
        .unwrap();
        self.slot.install(
            OpenKeys::pinning(keys, PinnedKeys::new()),
            testing::cache(&self.record, UnlockMethod::Passphrase),
        );
        self.operations = testing::native_operations(&self.slot, self.hub.clone());
    }
    fn request(&self) -> ChatMaterialRequest {
        let fingerprint = self
            .operations
            .current_opened_source()
            .unwrap()
            .with_current(|keys| {
                Ok(Fingerprint::from_bytes(
                    Sha256::digest(keys.keyset().public_key().to_bytes()).into(),
                ))
            })
            .unwrap();
        ChatMaterialRequest {
            stream: StreamBinding::new(
                AccountScope::new(
                    HubIdentity::new(
                        NamespaceId::new("trusted-hub").unwrap(),
                        Fingerprint::from_bytes([3; 32]),
                    ),
                    AccountIdentity::new(
                        NamespaceId::new(testing::ACCOUNT_ID).unwrap(),
                        fingerprint,
                    ),
                ),
                RunnerId::from_bytes([4; 16]),
                RunnerKeyEpoch::new(1).unwrap(),
                CloudChatId::from_bytes([5; 16]),
            ),
            epoch: ChatKeyEpoch::new(1).unwrap(),
            pins: ChatMaterialPins {
                encryption: [6; 32],
                signing: [7; 32],
                proof: [8; 32],
            },
        }
    }
}
fn reserve(bytes: usize) -> AllocationLease {
    PublicationBudget::maintenance(IntakePoolLimits::new(bytes, 1, 1).unwrap())
        .try_reserve(PublicationAllocation::new(bytes, 1).unwrap())
        .unwrap()
}
fn allocation() -> AllocationLease {
    reserve(account_material_work_bytes().unwrap())
}

async fn load(
    bound: &BoundOperations,
    request: ChatMaterialRequest,
    create: bool,
) -> Result<Allocated<record::MaterialRecord>, AccountMaterialError> {
    let allocation = allocation();
    let resident = reserve(ACCOUNT_MATERIAL_RESIDENT_BYTES);
    let work = bound.capture_bounded(MAX_RESPONSE, allocation)?;
    let record = storage::load_or_create(&work, request, create).await?;
    work.source()
        .publish(|| Allocated::new(record, resident))
        .map_err(AccountMaterialError::from)
}
fn journal(request: &ChatMaterialRequest) -> JournalCoordinate {
    JournalCoordinate::new(
        record::coordinate(request).unwrap(),
        JournalClass::Record,
        [9; 16],
        1,
        1,
        1,
    )
    .unwrap()
}

#[tokio::test]
async fn account_material_restores_on_a_second_normal_account_unlock_and_reuses_a_lost_reply() {
    let hub = Arc::new(MemoryHub::default());
    let (record, _) = testing::record(&testing::unlock(testing::PASSWORD));
    let a = Device::open(hub.clone(), record.clone());
    let request = a.request();
    let bound_a = a.bound();
    // A post-commit transport failure must reconcile the existing key, not put again.
    let (entered, release) = hub.pause_next_error(StoreBoundary::PutReady, HubError::NotSignedIn);
    let first = tokio::spawn(async move { load(&bound_a, request, true).await });
    entered.wait().await;
    release.release();
    let created = first.await.unwrap().unwrap();
    let request = created.value().request().clone();
    let slot = journal(&request);
    let ciphertext = storage::with_key(a.bound().source(), created.value(), |key| {
        key.seal_journal(&slot, b"original device content")
            .map_err(cause)
    })
    .unwrap();
    let b = Device::open(hub.clone(), record);
    let captured_a = a.bound();
    a.slot.lock();
    assert!(storage::with_key(captured_a.source(), created.value(), |_| Ok(())).is_err());
    let restored = load(&b.bound(), request.clone(), false).await.unwrap();
    let opened = storage::with_key(b.bound().source(), restored.value(), |key| {
        key.open_journal(&slot, &ciphertext).map_err(cause)
    })
    .unwrap();
    assert_eq!(&**opened, b"original device content");
    let repeated = load(&b.bound(), request.clone(), true).await.unwrap();
    assert_eq!(restored.value().wrapped(), repeated.value().wrapped());
    let at = b
        .bound()
        .capture_bounded(MAX_RESPONSE, allocation())
        .unwrap()
        .transient_store()
        .find(record::KIND)
        .await
        .unwrap()
        .unwrap();
    let objects = hub.objects(at.id());
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].version, 1);
    assert_eq!(objects[0].object_id, record::object_id(&request).unwrap());
    assert_ne!(objects[0].ciphertext, ciphertext);
}

#[tokio::test]
async fn account_material_rejects_substitution_oversize_and_stale_native_membership() {
    let hub = Arc::new(MemoryHub::default());
    let (record, _) = testing::record(&testing::unlock(testing::PASSWORD));
    let mut a = Device::open(hub.clone(), record);
    let request = a.request();
    let bound = a.bound();
    let original = load(&bound, request.clone(), true).await.unwrap();
    let mut changed = request.clone();
    changed.pins.signing[0] ^= 1;
    assert!(matches!(
        load(&bound, changed, true).await,
        Err(AccountMaterialError::Binding)
    ));
    let mut changed = request.clone();
    changed.epoch = ChatKeyEpoch::new(2).unwrap();
    assert!(matches!(
        load(&bound, changed, true).await,
        Err(AccountMaterialError::Rotation)
    ));
    let work = bound.capture_bounded(MAX_RESPONSE, allocation()).unwrap();
    let at = work.store().find(record::KIND).await.unwrap().unwrap();
    let id = record::object_id(&request).unwrap();
    let saved = hub.objects(at.id())[0].ciphertext.clone();
    hub.tamper(at.id(), &id, vec![0; record::MAX_RECORD + 43]);
    assert!(matches!(
        load(&bound, request.clone(), false).await,
        Err(AccountMaterialError::Capacity)
    ));
    let mut damaged = saved.clone();
    *damaged.last_mut().unwrap() ^= 1;
    hub.tamper(at.id(), &id, damaged);
    assert!(matches!(
        load(&bound, request.clone(), false).await,
        Err(AccountMaterialError::Backend(_))
    ));
    hub.tamper(at.id(), &id, saved);
    let (entered, release) = hub.pause_next(StoreBoundary::ObjectReady);
    let old_bound = bound.clone();
    let old_request = request.clone();
    let pending = tokio::spawn(async move { load(&old_bound, old_request, false).await });
    entered.wait().await;
    a.login();
    release.release();
    assert!(matches!(
        pending.await.unwrap(),
        Err(AccountMaterialError::Account(HubError::Stale))
    ));
    assert!(storage::with_key(bound.source(), original.value(), |_| Ok(())).is_err());
    let fresh = load(&a.bound(), request.clone(), false).await.unwrap();
    assert_eq!(fresh.value().wrapped(), original.value().wrapped());
    hub.delete_object(v1::DeleteObjectRequest {
        collection_id: at.id().to_owned(),
        object_id: id,
        expected_version: 1,
    })
    .await
    .unwrap();
    assert!(matches!(
        load(&a.bound(), request, true).await,
        Err(AccountMaterialError::Missing)
    ));
}

async fn add_tied_collection(device: &Device, id: &str, timestamp: i64) {
    let bound = device.bound();
    let (key, envelope) = bound
        .source()
        .native()
        .with_current(|keys| keys.generate_collection(id))
        .unwrap();
    let sealed_name = key.seal_name(id, record::KIND).unwrap();
    device
        .hub
        .create_collection(v1::CreateCollectionRequest {
            org_id: String::new(),
            kind: record::KIND.to_owned(),
            sealed_name,
            wrapped_key: envelope,
            collection_id: id.to_owned(),
        })
        .await
        .unwrap();
    device.hub.set_collection_created_at(id, timestamp);
}

#[tokio::test]
async fn account_material_bootstrap_uses_one_persisted_key_despite_tied_competing_collections() {
    let hub = Arc::new(MemoryHub::default());
    let (record, _) = testing::record(&testing::unlock(testing::PASSWORD));
    let a = Device::open(hub.clone(), record.clone());
    let b = Device::open(hub.clone(), record);
    let request = a.request();
    let expected = record::collection_id(request.stream.scope()).unwrap();
    let (entered, release) = hub.pause_next(StoreBoundary::CreateCollection);
    let bound_b = b.bound();
    let request_b = request.clone();
    let pending_b = tokio::spawn(async move { load(&bound_b, request_b, true).await });
    entered.wait().await;
    let a_material = load(&a.bound(), request.clone(), true).await.unwrap();
    let timestamp = hub
        .collections(record::KIND)
        .await
        .unwrap()
        .iter()
        .find(|row| row.collection_id == expected)
        .unwrap()
        .created_at_ms;
    // The already-started other device completes later. Both legitimate rows
    // tie the original timestamp, in opposite UUID/insertion order.
    add_tied_collection(&b, "ffffffff-ffff-ffff-ffff-ffffffffffff", timestamp).await;
    add_tied_collection(&b, "00000000-0000-0000-0000-000000000000", timestamp).await;
    release.release();
    let b_material = pending_b.await.unwrap().unwrap();
    assert_eq!(a_material.value().wrapped(), b_material.value().wrapped());
    let slot = journal(&request);
    let sealed = storage::with_key(a.bound().source(), a_material.value(), |key| {
        key.seal_journal(&slot, b"winner survives bootstrap")
            .map_err(cause)
    })
    .unwrap();
    let later = load(&b.bound(), request.clone(), false).await.unwrap();
    let plain = storage::with_key(b.bound().source(), later.value(), |key| {
        key.open_journal(&slot, &sealed).map_err(cause)
    })
    .unwrap();
    assert_eq!(&**plain, b"winner survives bootstrap");
    let rows = hub.collections(record::KIND).await.unwrap();
    assert_eq!(
        rows.iter()
            .filter(|row| row.collection_id == expected)
            .count(),
        1
    );
    assert_eq!(hub.objects(&expected).len(), 1);
    assert_eq!(
        rows.iter()
            .map(|row| hub.objects(&row.collection_id).len())
            .sum::<usize>(),
        1
    );
    let repeated = load(&b.bound(), request, true).await.unwrap();
    assert_eq!(repeated.value().wrapped(), a_material.value().wrapped());
}

#[test]
fn account_material_shared_error_cause_retains_its_actual_work_guard_and_typed_chain() {
    let budget = PublicationBudget::maintenance(IntakePoolLimits::new(1024, 1, 1).unwrap());
    let allocation = budget
        .try_reserve(PublicationAllocation::new(1024, 1).unwrap())
        .unwrap();
    let original = SharedCause::new(std::io::Error::other("foreign metadata"));
    let error = retain_error(AccountMaterialError::Backend(original), allocation);
    let public = SharedCause::new(error);
    let mut current: &(dyn std::error::Error + 'static) = public.as_ref();
    let mut shared = None;
    let mut leaf = false;
    loop {
        if let Some(cause) = current.downcast_ref::<SharedCause>() {
            shared = Some(cause.clone());
        }
        leaf |= current.downcast_ref::<std::io::Error>().is_some();
        match current.source() {
            Some(next) => current = next,
            None => break,
        }
    }
    assert!(leaf);
    let shared = shared.unwrap();
    drop(public);
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(1024, 1).unwrap())
            .is_err()
    );
    drop(shared);
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(1024, 1).unwrap())
            .is_ok()
    );
}

async fn grant(
    bound: &BoundOperations,
    request: &ChatMaterialRequest,
    make: impl FnOnce() -> Result<grant::GrantRecord, AccountMaterialError>,
) -> Result<grant::GrantRecord, AccountMaterialError> {
    let work = bound.capture_bounded(MAX_RESPONSE, allocation())?;
    grant::load_or_create(&work, request, make).await
}
fn sealed(byte: u8) -> Result<grant::GrantRecord, AccountMaterialError> {
    grant::GrantRecord::new([byte; 16], vec![byte; 200])
}

#[tokio::test]
async fn a_chat_key_grant_is_stored_once_and_every_retry_and_device_reuses_its_bytes() {
    let hub = Arc::new(MemoryHub::default());
    let (record, _) = testing::record(&testing::unlock(testing::PASSWORD));
    let a = Device::open(hub.clone(), record.clone());
    let b = Device::open(hub.clone(), record);
    let request = a.request();
    let first = grant(&a.bound(), &request, || sealed(1)).await.unwrap();
    let mut sealed_again = false;
    let retried = grant(&a.bound(), &request, || {
        sealed_again = true;
        sealed(2)
    })
    .await
    .unwrap();
    let other_device = grant(&b.bound(), &request, || sealed(3)).await.unwrap();
    assert!(!sealed_again);
    assert_eq!(retried, first);
    assert_eq!(other_device, first);
    let collection = record::collection_id_of(request.stream.scope(), grant::KIND).unwrap();
    assert_eq!(hub.objects(&collection).len(), 1);
}

#[tokio::test]
async fn a_chat_key_grant_keeps_its_own_collection_apart_from_the_chat_key() {
    let hub = Arc::new(MemoryHub::default());
    let (record, _) = testing::record(&testing::unlock(testing::PASSWORD));
    let a = Device::open(hub.clone(), record);
    let request = a.request();
    load(&a.bound(), request.clone(), true).await.unwrap();
    grant(&a.bound(), &request, || sealed(1)).await.unwrap();
    let material = record::collection_id(request.stream.scope()).unwrap();
    let grants = record::collection_id_of(request.stream.scope(), grant::KIND).unwrap();
    assert_ne!(material, grants);
    assert_eq!(hub.objects(&material).len(), 1);
    assert_eq!(hub.objects(&grants).len(), 1);
}

#[tokio::test]
async fn a_lost_acknowledgement_of_the_grant_write_resolves_to_the_stored_bytes() {
    let hub = Arc::new(MemoryHub::default());
    let (record, _) = testing::record(&testing::unlock(testing::PASSWORD));
    let a = Device::open(hub.clone(), record);
    let request = a.request();
    let bound = a.bound();
    let (entered, release) = hub.pause_next_error(StoreBoundary::PutReady, HubError::NotSignedIn);
    let pending_request = request.clone();
    let pending_bound = bound.clone();
    let first =
        tokio::spawn(async move { grant(&pending_bound, &pending_request, || sealed(1)).await });
    entered.wait().await;
    release.release();
    let outcome = first.await.unwrap().unwrap();
    assert_eq!(outcome, sealed(1).unwrap());
    let retried = grant(&bound, &request, || sealed(9)).await.unwrap();
    assert_eq!(retried, outcome);
}

#[tokio::test]
async fn a_damaged_or_oversize_chat_key_grant_is_refused_rather_than_republished() {
    let hub = Arc::new(MemoryHub::default());
    let (record, _) = testing::record(&testing::unlock(testing::PASSWORD));
    let a = Device::open(hub.clone(), record);
    let request = a.request();
    let bound = a.bound();
    grant(&bound, &request, || sealed(1)).await.unwrap();
    let collection = record::collection_id_of(request.stream.scope(), grant::KIND).unwrap();
    let id = record::object_id(&request).unwrap();
    let saved = hub.objects(&collection)[0].ciphertext.clone();
    hub.tamper(&collection, &id, vec![0; grant::MAX_RECORD + 43]);
    assert!(matches!(
        grant(&bound, &request, || sealed(2)).await,
        Err(AccountMaterialError::Capacity)
    ));
    let mut damaged = saved;
    *damaged.last_mut().unwrap() ^= 1;
    hub.tamper(&collection, &id, damaged);
    assert!(matches!(
        grant(&bound, &request, || sealed(2)).await,
        Err(AccountMaterialError::Backend(_))
    ));
}
