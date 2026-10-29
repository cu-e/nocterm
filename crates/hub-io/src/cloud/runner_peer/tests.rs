//! Connection setup validates native pins before any remote or secret publication.
use super::*;
use nocterm_cloud_model::{
    coordinator::{IntakeLane, IntakeLimits},
    journal::PublicationBudget,
};
use nocterm_hubs_model::{HubAddress, TrustPolicy};

#[test]
fn runner_hub_setup_requires_original_authority_namespace_identity_and_private_owner() {
    let directory = tempfile::tempdir().unwrap();
    let authority = DeviceKey::generate().unwrap().public_key();
    let account = nocterm_keys::AccountKeyPair::generate().unwrap();
    let endpoint = || HubEndpointSpec {
        address: HubAddress::parse("localhost:7443").unwrap(),
        trust: TrustPolicy::PublicCa,
    };
    let binding = RunnerHubTrust::new(endpoint(), "original-hub".into(), authority)
        .binding(
            "account",
            account.public_key().to_bytes(),
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(4).unwrap(),
        )
        .unwrap();
    let identity = Arc::new(
        CloudRunnerIdentity::create_new(&directory.path().join("identity"), binding.clone())
            .unwrap(),
    );
    let budget = Arc::new(PublicationBudget::maintenance(
        IntakeLimits::default().pool(IntakeLane::Ordinary),
    ));
    let private = directory.path().join("session");
    assert!(matches!(
        RunnerHubConnection::new(
            RunnerHubTrust::new(endpoint(), "original-hub".into(), [9; 32]),
            binding.clone(),
            identity.clone(),
            &private,
            budget.clone()
        ),
        Err(RunnerHubError::Binding)
    ));
    assert!(!private.exists());
    assert!(matches!(
        RunnerHubConnection::new(
            RunnerHubTrust::new(endpoint(), "other-hub".into(), authority),
            binding.clone(),
            identity.clone(),
            &private,
            budget.clone()
        ),
        Err(RunnerHubError::Binding)
    ));
    let connection = RunnerHubConnection::new(
        RunnerHubTrust::new(endpoint(), "original-hub".into(), authority),
        binding.clone(),
        identity.clone(),
        &private,
        budget.clone(),
    )
    .unwrap();
    assert!(
        RunnerHubConnection::new(
            RunnerHubTrust::new(endpoint(), "original-hub".into(), authority),
            binding,
            identity.clone(),
            &private,
            budget
        )
        .is_err()
    );
    assert!(connection.peer().is_err());
    assert!(!private.join("runner-session.v1").exists());
    identity.retire();
    assert!(matches!(
        connection.peer(),
        Err(RunnerHubError::Identity(_))
    ));
    connection.retire_local();
    assert!(matches!(
        connection.peer(),
        Err(RunnerHubError::Unavailable)
    ));
}

#[test]
fn runner_hub_failures_expose_native_unsupported_and_original_source_errors() {
    use std::error::Error;
    let unsupported = sdk_error(CloudError::Unsupported);
    assert!(matches!(unsupported, RunnerHubError::Unsupported(_)));
    assert!(unsupported.source().is_some());
    let retired = sdk_error(CloudError::SourceChanged);
    assert!(matches!(retired, RunnerHubError::SourceChanged(_)));
    assert!(retired.source().is_some());
}
