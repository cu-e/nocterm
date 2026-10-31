//! Real native account membership and actual installed R keys; no new RPC fake.
use super::*;
use crate::{
    account::{
        Accounts, Slot,
        operations::{AccountOperations, BoundOperations},
        testing,
    },
    store::memory::MemoryHub,
};
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    identity::*,
    journal::{PublicationAllocation, PublicationBudget, PublicationWorkBudget},
    testing::ManualToolTimer,
};
use nocterm_hub_model::cloud::provider::{
    BootDescriptor, BootWindow, ProviderLeaseContext, SignedBootDescriptor,
};
use nocterm_hubs_model::{HubAddress, HubEndpointSpec, TrustPolicy};
use nocterm_keys::{DeviceKey, cloud::RunnerBootKey};
use sha2::{Digest, Sha256};

fn reserve(bytes: usize) -> AllocationLease {
    PublicationBudget::maintenance(IntakePoolLimits::new(bytes, 1, 1).unwrap())
        .try_reserve(PublicationAllocation::new(bytes, 1).unwrap())
        .unwrap()
}
struct Fixture {
    _directory: tempfile::TempDir,
    _accounts: Accounts,
    slot: Arc<Slot>,
    operations: Arc<AccountOperations>,
    subjects: Arc<testing::ScriptedSubjects>,
    bound: BoundOperations,
    binding: JournalCatalogBinding,
    identity: Arc<super::super::CloudRunnerIdentity>,
    timer: Arc<ManualToolTimer>,
    pins: super::super::ChatMaterialPins,
    record: nocterm_hub_proto::v1::AccountKeyset,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (accounts, slot, record) = testing::signed_in(directory.path());
        let (operations, subjects) =
            testing::native_operations_with_subjects(&slot, Arc::new(MemoryHub::default()));
        let bound = operations
            .bind(operations.current_opened_source().unwrap())
            .unwrap();
        let account = bound
            .source()
            .native()
            .with_current(|keys| Ok(keys.keyset().public_key().to_bytes()))
            .unwrap();
        let trust = super::super::RunnerHubTrust::new(
            HubEndpointSpec {
                address: HubAddress::parse("https://localhost:8443").unwrap(),
                trust: TrustPolicy::PublicCa,
            },
            "provider-test-hub".into(),
            DeviceKey::generate().unwrap().public_key(),
        );
        let binding = trust
            .binding(
                testing::ACCOUNT_ID,
                account,
                RunnerId::from_bytes([2; 16]),
                RunnerKeyEpoch::new(3).unwrap(),
            )
            .unwrap();
        let identity = Arc::new(
            super::super::CloudRunnerIdentity::create_new(
                &directory.path().join("identity"),
                binding.clone(),
            )
            .unwrap(),
        );
        let public = identity.public_identity().unwrap();
        let pins = super::super::ChatMaterialPins {
            encryption: *public.encryption_key(),
            signing: *public.signing_key(),
            proof: *public.proof_key(),
        };
        let timer = Arc::new(ManualToolTimer::default());
        timer.set(110_000);
        Self {
            _directory: directory,
            _accounts: accounts,
            slot,
            operations,
            subjects,
            bound,
            binding,
            identity,
            timer,
            pins,
            record,
        }
    }
    fn descriptor(&self) -> BootDescriptor {
        BootDescriptor::new(
            super::super::metadata_crypto::namespace(&self.binding).unwrap(),
            super::super::metadata_crypto::runner(&self.binding),
            [4; 16],
            RunnerBootKey::generate().unwrap().public_key().to_bytes(),
            BootWindow {
                issued_at_ms: 100_000,
                expires_at_ms: 160_000,
            },
        )
        .unwrap()
    }
    fn signed(&self, descriptor: BootDescriptor) -> SignedBootDescriptor {
        self.identity
            .with_keys(|keys| keys.signing().attest_boot(descriptor))
            .unwrap()
            .unwrap()
    }
    fn verifier(&self, bound: BoundOperations) -> BootVerifier {
        BootVerifier::new(
            bound,
            self.binding.clone(),
            self.pins,
            self.timer.clone(),
            reserve(PROVIDER_BOOT_RESIDENT_BYTES),
        )
        .unwrap()
    }
}
#[tokio::test]
async fn provider_boot_uses_installed_record_signer_and_preserves_original_account_window() {
    let fixture = Fixture::new();
    let signed = fixture.signed(fixture.descriptor());
    let work = reserve(provider_boot_work_bytes());
    let bytes = announcement_body(&signed, &work).unwrap();
    assert!(bytes.len() <= 580);
    assert!(matches!(
        announcement_body(&signed, &reserve(provider_boot_work_bytes() - 1)),
        Err(ProviderBootError::Capacity)
    ));
    let verifier = fixture.verifier(fixture.bound.clone());
    let opened = verifier.open(&bytes, work).unwrap();
    assert_eq!(opened.value().descriptor(), signed.descriptor());
    let proof_pins = super::super::ChatMaterialPins {
        signing: fixture.pins.proof,
        ..fixture.pins
    };
    let wrong = BootVerifier::new(
        fixture.bound.clone(),
        fixture.binding.clone(),
        proof_pins,
        fixture.timer.clone(),
        reserve(PROVIDER_BOOT_RESIDENT_BYTES),
    )
    .unwrap();
    assert!(
        wrong
            .open(&bytes, reserve(provider_boot_work_bytes()))
            .is_err()
    );
    let mut namespace = signed.descriptor().namespace().clone();
    namespace.account_id = nocterm_hub_model::cloud::CloudTextId::new("other-account").unwrap();
    let other = BootDescriptor::new(
        namespace,
        signed.descriptor().runner(),
        [4; 16],
        signed.descriptor().recipient(),
        signed.descriptor().window(),
    )
    .unwrap();
    assert!(matches!(
        verifier.open(
            &fixture.signed(other).canonical_bytes().unwrap(),
            reserve(provider_boot_work_bytes())
        ),
        Err(ProviderBootError::Binding)
    ));
    let mut runner = signed.descriptor().runner();
    runner.key_epoch += 1;
    let other = BootDescriptor::new(
        signed.descriptor().namespace().clone(),
        runner,
        [4; 16],
        signed.descriptor().recipient(),
        signed.descriptor().window(),
    )
    .unwrap();
    assert!(matches!(
        verifier.open(
            &fixture.signed(other).canonical_bytes().unwrap(),
            reserve(provider_boot_work_bytes())
        ),
        Err(ProviderBootError::Binding)
    ));
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(
        verifier
            .open(&corrupt, reserve(provider_boot_work_bytes()))
            .is_err()
    );
    fixture.timer.set(160_000);
    assert!(matches!(
        verifier.open(&bytes, reserve(provider_boot_work_bytes())),
        Err(ProviderBootError::Expired)
    ));
}
#[tokio::test]
async fn provider_boot_keeps_actual_quota_and_never_revives_after_clock_or_login_replacement() {
    let fixture = Fixture::new();
    let verifier = fixture.verifier(fixture.bound.clone());
    let bytes = fixture
        .signed(fixture.descriptor())
        .canonical_bytes()
        .unwrap();
    let required = provider_boot_work_bytes();
    let budget = PublicationBudget::maintenance(IntakePoolLimits::new(required, 1, 1).unwrap());
    let original = budget
        .try_reserve(PublicationAllocation::new(required, 1).unwrap())
        .unwrap();
    let opened = verifier.open(&bytes, original).unwrap();
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(required, 1).unwrap())
            .is_err()
    );
    drop(opened);
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(required, 1).unwrap())
            .is_ok()
    );
    assert!(matches!(
        verifier.open(&bytes, reserve(required - 1)),
        Err(ProviderBootError::Capacity)
    ));
    assert!(matches!(
        verifier.open(&vec![0; 581], reserve(required)),
        Err(ProviderBootError::Capacity)
    ));
    fixture.timer.set(90_000);
    assert!(matches!(
        verifier.open(&bytes, reserve(required)),
        Err(ProviderBootError::Clock)
    ));
    fixture.timer.set(120_000);
    assert!(matches!(
        verifier.open(&bytes, reserve(required)),
        Err(ProviderBootError::Clock)
    ));
    let original = fixture.verifier(fixture.bound.clone());
    fixture.slot.lock();
    fixture
        .subjects
        .sign_in(testing::hub(), "another-account", "device-b");
    fixture
        .subjects
        .sign_in(testing::hub(), testing::ACCOUNT_ID, "device");
    fixture.slot.install(
        testing::open(&fixture.record, &testing::unlock(testing::PASSWORD)),
        testing::cache(&fixture.record, crate::cache::UnlockMethod::Passphrase),
    );
    assert!(original.open(&bytes, reserve(required)).is_err());
    let fresh = fixture
        .operations
        .bind(fixture.operations.current_opened_source().unwrap())
        .unwrap();
    assert!(
        fixture
            .verifier(fresh)
            .open(&bytes, reserve(required))
            .is_ok()
    );
    // The trusted selected fingerprint really came from the opened native keyset.
    let actual = fixture
        .bound
        .source()
        .native()
        .with_current(|keys| Ok(Sha256::digest(keys.keyset().public_key().to_bytes())));
    assert!(actual.is_err());
}
#[tokio::test]
async fn a_sealed_lease_opens_only_on_the_runner_boot_that_was_verified() {
    let fixture = Fixture::new();
    let boot = RunnerBootKey::generate().unwrap();
    let descriptor = BootDescriptor::new(
        super::super::metadata_crypto::namespace(&fixture.binding).unwrap(),
        super::super::metadata_crypto::runner(&fixture.binding),
        [4; 16],
        boot.public_key().to_bytes(),
        BootWindow {
            issued_at_ms: 100_000,
            expires_at_ms: 160_000,
        },
    )
    .unwrap();
    let bytes = fixture.signed(descriptor).canonical_bytes().unwrap();
    let verifier = fixture.verifier(fixture.bound.clone());
    let verified = verifier
        .open(&bytes, reserve(provider_boot_work_bytes()))
        .unwrap();
    let secret = b"sk-device-secret-0123456789";
    let request = LeaseRequest {
        provider: "openai",
        lease_id: [5; 16],
        expires_ms: 400_000,
    };
    let sealed = verifier.seal(verified.value(), &request, secret).unwrap();
    // A boot that ends sooner shortens the lease.
    assert_eq!(sealed.expires().as_millis(), 160_000);
    assert!(!format!("{sealed:?}").contains("sk-device"));
    assert!(
        !sealed
            .capsule()
            .windows(secret.len())
            .any(|window| window == secret)
    );
    let account = fixture
        .bound
        .source()
        .native()
        .with_current(|keys| Ok(keys.keyset().public_key()))
        .unwrap();
    let context = ProviderLeaseContext::from_canonical_bytes(sealed.context()).unwrap();
    let opened = boot
        .open_provider_lease(&account, &context, sealed.capsule(), 110_000)
        .unwrap();
    assert_eq!(opened.secret(), secret);
    let other = RunnerBootKey::generate().unwrap();
    assert!(
        other
            .open_provider_lease(&account, &context, sealed.capsule(), 110_000)
            .is_err()
    );
    // Sealing is refused once the boot window is over.
    fixture.timer.set(160_000);
    assert!(verifier.seal(verified.value(), &request, secret).is_err());
}
