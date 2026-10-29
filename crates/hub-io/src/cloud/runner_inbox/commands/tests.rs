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

#[test]
fn authenticated_stop_keeps_original_id_run_and_signed_header_digest() {
    let account = AccountKeyPair::generate().unwrap();
    let runner = RunnerEncryptionKey::generate().unwrap();
    let device = DeviceKey::generate().unwrap();
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
