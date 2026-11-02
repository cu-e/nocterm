//! Real SDK account boxes/signatures retain the original native command identity.
use super::*;
use nocterm_hub_model::cloud::{
    CloudNamespace, CloudRoute, CloudTextId, CommandContext, CommandHeader, CommandKeys,
    RunCoordinate, RunnerCoordinate,
};
use nocterm_keys::{
    AccountKeyPair, DeviceKey,
    cloud::{RunnerEncryptionKey, open_command, seal_command},
};

fn stream_and_destination(account: &AccountKeyPair) -> (StreamBinding, RunDestination) {
    let source = StreamBinding::new(
        AccountScope::new(
            HubIdentity::new(
                NamespaceId::new("hub").unwrap(),
                Fingerprint::from_bytes([1; 32]),
            ),
            AccountIdentity::new(
                NamespaceId::new("account").unwrap(),
                Fingerprint::from_bytes(Sha256::digest(account.public_key().as_bytes()).into()),
            ),
        ),
        RunnerId::from_bytes([2; 16]),
        RunnerKeyEpoch::new(3).unwrap(),
        CloudChatId::from_bytes([4; 16]),
    );
    let target = TargetBinding::new(
        Fingerprint::from_bytes([5; 32]),
        Fingerprint::from_bytes([6; 32]),
        NamespaceId::new("principal").unwrap(),
        Fingerprint::from_bytes([7; 32]),
    );
    let authentication = AuthenticationBinding::new(
        Fingerprint::from_bytes([8; 32]),
        Fingerprint::from_bytes([9; 32]),
    );
    let default = RunDestination::new(target, authentication, None).unwrap();
    (source, default)
}

#[test]
fn authenticated_stop_keeps_original_id_run_and_signed_header_digest() {
    let account = AccountKeyPair::generate().unwrap();
    let runner = RunnerEncryptionKey::generate().unwrap();
    let device = DeviceKey::generate().unwrap();
    let (source, default) = stream_and_destination(&account);
    let route = CloudRoute {
        namespace: CloudNamespace {
            hub_instance_id: CloudTextId::new("hub").unwrap(),
            account_id: CloudTextId::new("account").unwrap(),
        },
        runner: RunnerCoordinate {
            id: [2; 16],
            key_epoch: 3,
        },
        chat_id: [4; 16],
        command_id: [10; 16],
    };
    let payload = wire::CloudCommandPayload::Stop.canonical_bytes().unwrap();
    let header = CommandHeader::new(
        route.clone(),
        CommandKeys {
            account: account.public_key().to_bytes(),
            runner: runner.public_key().to_bytes(),
        },
        nocterm_hub_model::cloud::CommandIssuer {
            device_id: CloudTextId::new("device").unwrap(),
            signing_key: device.public_key(),
            access_token_digest: [11; 32],
        },
        CommandContext {
            nonce: [12; 16],
            expected_chat_revision: 13,
            run: Some(RunCoordinate {
                id: [14; 16],
                expected_epoch: 15,
            }),
            kind: nocterm_hub_model::cloud::CommandKind::Stop,
            issued_at_ms: 100_000,
            accept_before_ms: 160_000,
        },
        &payload,
    )
    .unwrap();
    let canonical = header.canonical_bytes().unwrap();
    let signed = SignedCommand::new(header, &payload, device.sign(&canonical)).unwrap();
    let ciphertext = seal_command(&account, &runner.public_key(), &signed).unwrap();
    let opened = open_command(&runner, &account.public_key(), &route, &ciphertext).unwrap();
    let original = opened.command().unwrap();
    let acceptance = acceptance(&original).unwrap();
    let mapped = map(
        &original,
        wire::CloudCommandPayload::from_command(&original).unwrap(),
        source.clone(),
        acceptance.clone(),
        &default,
    )
    .unwrap();
    assert_eq!(mapped.command.stream(), &source);
    assert_eq!(
        mapped.command.acceptance().id(),
        CommandId::from_bytes([10; 16])
    );
    assert_eq!(
        acceptance.digest(),
        Fingerprint::from_bytes(Sha256::digest(&canonical).into())
    );
    assert_eq!(
        mapped.command.run().unwrap().run(),
        RunId::from_bytes([14; 16])
    );
    assert_eq!(
        mapped.command.run().unwrap().epoch(),
        RunEpoch::new(15).unwrap()
    );
    assert!(matches!(mapped.command.body(), CommandBody::Stop));
    assert!(mapped.attachment.is_none());
    let mut moved = route;
    moved.command_id[0] ^= 1;
    assert!(open_command(&runner, &account.public_key(), &moved, &ciphertext).is_err());
}

fn grant_route() -> CloudRoute {
    CloudRoute {
        namespace: CloudNamespace {
            hub_instance_id: CloudTextId::new("hub").unwrap(),
            account_id: CloudTextId::new("account").unwrap(),
        },
        runner: RunnerCoordinate {
            id: [2; 16],
            key_epoch: 3,
        },
        chat_id: [4; 16],
        command_id: [10; 16],
    }
}

#[test]
fn a_device_inference_grant_maps_to_the_run_it_names_with_the_signed_bounds() {
    let account = AccountKeyPair::generate().unwrap();
    let runner = RunnerEncryptionKey::generate().unwrap();
    let device = DeviceKey::generate().unwrap();
    let (source, default) = stream_and_destination(&account);
    let route = grant_route();
    let payload = wire::CloudCommandPayload::IssueGrant(Box::new(wire::GrantPayload::Inference {
        grant: wire::GrantReference {
            id: [20; 16],
            epoch: 1,
        },
        model: wire::ModelSelection {
            provider: CloudTextId::new("managed").unwrap(),
            model: CloudTextId::new("model-a").unwrap(),
            pricing_policy: CloudTextId::new("managed-v1").unwrap(),
        },
        window: wire::GrantWindow {
            issued_at_ms: 100_000,
            expires_at_ms: 150_000,
        },
        limits: wire::InferenceLimits {
            calls: 8,
            turns: 4,
            request_bytes: 1024,
            output_bytes: 2048,
            output_tokens: 512,
            spend_micros: 99,
            deadline_ms: 150_000,
        },
    }))
    .canonical_bytes()
    .unwrap();
    let header = CommandHeader::new(
        route.clone(),
        CommandKeys {
            account: account.public_key().to_bytes(),
            runner: runner.public_key().to_bytes(),
        },
        nocterm_hub_model::cloud::CommandIssuer {
            device_id: CloudTextId::new("device").unwrap(),
            signing_key: device.public_key(),
            access_token_digest: [11; 32],
        },
        CommandContext {
            nonce: [12; 16],
            expected_chat_revision: 13,
            run: Some(RunCoordinate {
                id: [14; 16],
                expected_epoch: 1,
            }),
            kind: nocterm_hub_model::cloud::CommandKind::IssueGrant,
            issued_at_ms: 100_000,
            accept_before_ms: 160_000,
        },
        &payload,
    )
    .unwrap();
    let canonical = header.canonical_bytes().unwrap();
    let signed = SignedCommand::new(header, &payload, device.sign(&canonical)).unwrap();
    let ciphertext = seal_command(&account, &runner.public_key(), &signed).unwrap();
    let opened = open_command(&runner, &account.public_key(), &route, &ciphertext).unwrap();
    let original = opened.command().unwrap();
    let mapped = map(
        &original,
        wire::CloudCommandPayload::from_command(&original).unwrap(),
        source,
        acceptance(&original).unwrap(),
        &default,
    )
    .unwrap();
    let CommandBody::InferenceGrant(grant) = mapped.command.body() else {
        panic!("a signed inference grant maps to the runner's inference grant");
    };
    assert_eq!(grant.id(), GrantId::from_bytes([20; 16]));
    assert_eq!(grant.allowance().run().run(), RunId::from_bytes([14; 16]));
    assert_eq!(grant.allowance().spend_micros(), 99);
    assert_eq!(grant.allowance().budget().turns(), 4);
    assert_eq!(grant.allowance().window().expires().as_millis(), 150_000);
    assert!(mapped.attachment.is_none());
}
