//! Actual account-DH boot capsules reach a receiver only under their original reservation.
use super::*;
use nocterm_cloud_model::{
    coordinator::{CommandReceiveBudget, IntakeLane, IntakeLimits},
    testing::RecordingProviderCredentialReceiver,
};
use nocterm_hub_model::cloud::{
    CloudNamespace, CloudTextId, RunnerCoordinate,
    provider::{BootDescriptor, BootWindow},
};
use nocterm_keys::{
    AccountKeyPair,
    cloud::{RunnerSigningKey, seal_provider_lease},
};

fn capsule_fixture() -> (AccountKeyPair, RunnerBootKey, ProviderLeaseContext, Vec<u8>) {
    let account = AccountKeyPair::generate().unwrap();
    let boot = RunnerBootKey::generate().unwrap();
    let signing = RunnerSigningKey::from_seed(&[1; 32]);
    let descriptor = BootDescriptor::new(
        CloudNamespace {
            hub_instance_id: CloudTextId::new("hub").unwrap(),
            account_id: CloudTextId::new("account").unwrap(),
        },
        RunnerCoordinate {
            id: [2; 16],
            key_epoch: 3,
        },
        [4; 16],
        boot.public_key().to_bytes(),
        BootWindow {
            issued_at_ms: 100_000,
            expires_at_ms: 160_000,
        },
    )
    .unwrap();
    let signed = signing.attest_boot(descriptor).unwrap();
    let verified = signed
        .verify(
            &signing.public_key(),
            signed.descriptor().namespace(),
            signed.descriptor().runner(),
            110_000,
        )
        .unwrap();
    let context = ProviderLeaseContext::new(
        signed.descriptor().clone(),
        CloudTextId::new("openai").unwrap(),
        [5; 16],
        150_000,
    )
    .unwrap();
    let capsule = seal_provider_lease(
        &account,
        &verified,
        &context,
        b"actual-fixture-secret",
        110_000,
    )
    .unwrap();
    (account, boot, context, capsule)
}

#[test]
fn provider_bridge_opens_only_original_account_boot_and_retains_the_actual_key_quota() {
    let (account, boot, context, capsule) = capsule_fixture();
    let budget = CommandReceiveBudget::new(IntakeLimits::default());
    let reservation = || {
        budget
            .try_reserve(IntakeLane::Ordinary, 64 * 1024, 1)
            .unwrap()
    };
    let mut receiver = RecordingProviderCredentialReceiver::default();
    let mut damaged = capsule.clone();
    *damaged.last_mut().unwrap() ^= 1;
    assert!(
        open_into(
            &boot,
            &account.public_key(),
            &context,
            &damaged,
            Timestamp::from_millis(110_000),
            reservation(),
            &mut receiver
        )
        .is_err()
    );
    assert!(receiver.received().is_none());
    let new_boot = RunnerBootKey::generate().unwrap();
    assert!(
        open_into(
            &new_boot,
            &account.public_key(),
            &context,
            &capsule,
            Timestamp::from_millis(110_000),
            reservation(),
            &mut receiver
        )
        .is_err()
    );
    let other_account = AccountKeyPair::generate().unwrap();
    assert!(
        open_into(
            &boot,
            &other_account.public_key(),
            &context,
            &capsule,
            Timestamp::from_millis(110_000),
            reservation(),
            &mut receiver
        )
        .is_err()
    );
    assert!(
        open_into(
            &boot,
            &account.public_key(),
            &context,
            &capsule,
            Timestamp::from_millis(150_000),
            reservation(),
            &mut receiver
        )
        .is_err()
    );
    assert_eq!(budget.usage(IntakeLane::Ordinary).unwrap().bytes, 0);
    open_into(
        &boot,
        &account.public_key(),
        &context,
        &capsule,
        Timestamp::from_millis(110_000),
        reservation(),
        &mut receiver,
    )
    .unwrap();
    let received = receiver.received().unwrap();
    assert_eq!(received.value(), b"actual-fixture-secret");
    assert_eq!(received.allocation().bytes(), 64 * 1024);
    assert!(
        open_into(
            &boot,
            &account.public_key(),
            &context,
            &capsule,
            Timestamp::from_millis(110_000),
            reservation(),
            &mut receiver
        )
        .is_err()
    );
    assert_eq!(budget.usage(IntakeLane::Ordinary).unwrap().bytes, 64 * 1024);
    drop(receiver);
    assert_eq!(budget.usage(IntakeLane::Ordinary).unwrap().bytes, 0);
}
