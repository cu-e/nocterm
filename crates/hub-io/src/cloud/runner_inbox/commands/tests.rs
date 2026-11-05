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
        None,
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
        None,
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

mod ssh_grants {
    use super::*;
    use nocterm_cloud_model::ssh_access::{CertificateFacts as Facts, SshCertificateReader};
    use nocterm_hub_model::cloud::{
        ChatCoordinate,
        ssh::{BindingWindow, SshBinding, SshServerId, SshTarget},
    };
    use nocterm_keys::agents::ssh_ca::{CertificateSpec, UserCa, principal_for};

    const CA_SEED: [u8; 32] = [21; 32];

    fn signed(key_epoch: u64) -> nocterm_hub_model::cloud::ssh::SignedSshBinding {
        let ca = UserCa::from_seed(CA_SEED);
        let chat = ChatCoordinate::new(
            CloudNamespace {
                hub_instance_id: CloudTextId::new("hub").unwrap(),
                account_id: CloudTextId::new("account").unwrap(),
            },
            RunnerCoordinate {
                id: [2; 16],
                key_epoch: 3,
            },
            [4; 16],
            key_epoch,
        )
        .unwrap();
        ca.sign_binding(SshBinding {
            chat,
            grant: wire::GrantReference {
                id: [30; 16],
                epoch: 2,
            },
            window: BindingWindow {
                issued_at_ms: 1_000,
                expires_at_ms: 1_000 + 3_600_000,
            },
            ca_key: ca.public_key(),
            targets: vec![SshTarget {
                server: SshServerId::of_profile("profile-1"),
                host: CloudTextId::new("db.example").unwrap(),
                port: 2222,
                user: CloudTextId::new("deploy").unwrap(),
                host_key_sha256: [31; 32],
            }],
        })
        .unwrap()
    }

    #[test]
    fn a_binding_for_this_chat_key_names_each_server_with_its_own_principal() {
        let account = AccountKeyPair::generate().unwrap();
        let (stream, _) = stream_and_destination(&account);
        let body = ssh::binding(&signed(5), &stream, Some(ChatKeyEpoch::new(5).unwrap())).unwrap();
        let CommandBody::SshBinding(offer) = body else {
            panic!("not a binding");
        };
        let server = SshServerId::of_profile("profile-1");
        let target = &offer.targets()[0];
        assert_eq!(target.server.as_bytes(), server.as_bytes());
        assert_eq!(
            target.principal,
            principal_for(&UserCa::from_seed(CA_SEED).public_key(), &server)
        );
        assert_eq!(
            (target.endpoint.host.as_str(), target.endpoint.port),
            ("db.example", 2222)
        );
        assert_eq!(offer.grant().class(), GrantClass::SshBinding);
        assert_eq!(
            offer.signed().as_bytes(),
            signed(5).canonical_bytes().unwrap()
        );
    }

    #[test]
    fn a_bound_server_carries_the_session_access_a_prompt_names_for_it() {
        let account = AccountKeyPair::generate().unwrap();
        let (stream, _) = stream_and_destination(&account);
        let body = ssh::binding(&signed(5), &stream, Some(ChatKeyEpoch::new(5).unwrap())).unwrap();
        let CommandBody::SshBinding(offer) = body else {
            panic!("not a binding");
        };
        let server = SshServerId::of_profile("profile-1");
        let prompt = signed(5).binding().target_access(&server).unwrap();
        let access = offer.targets()[0].access.clone().unwrap();
        // A prompt's target maps through the same functions as `submit` does.
        assert_eq!(access.target, super::target(&prompt.target).unwrap());
        assert_eq!(
            access.authentication,
            super::authentication(prompt.authentication)
        );
        assert_eq!(access.target.host().as_bytes(), &[31; 32]);
        assert_eq!(access.target.principal().as_str(), "deploy");
        // A newer epoch of the same grant names another credential revision.
        let mut newer = signed(5).binding().clone();
        newer.grant.epoch += 1;
        let newer = UserCa::from_seed(CA_SEED).sign_binding(newer).unwrap();
        let moved = newer.binding().target_access(&server).unwrap();
        assert_ne!(
            access.authentication,
            super::authentication(moved.authentication)
        );
    }

    #[test]
    fn a_binding_for_another_chat_key_epoch_is_refused() {
        let account = AccountKeyPair::generate().unwrap();
        let (stream, _) = stream_and_destination(&account);
        for epoch in [None, Some(ChatKeyEpoch::new(6).unwrap())] {
            assert!(matches!(
                ssh::binding(&signed(5), &stream, epoch),
                Err(CoordinatorError::InvalidCommand)
            ));
        }
    }

    #[test]
    fn a_certificate_of_the_binding_ca_is_read_and_another_ca_is_refused() {
        let ca = UserCa::from_seed(CA_SEED);
        let spec = CertificateSpec {
            principals: vec![principal_for(
                &ca.public_key(),
                &SshServerId::of_profile("p"),
            )],
            key_id: "nocterm-chat-test".into(),
            subject_key: [40; 32],
            serial: 7,
            valid_after_ms: 1_700_000_000_000,
            valid_before_ms: 1_700_000_000_000 + 3_600_000,
            source_addresses: vec!["203.0.113.7".into()],
        };
        let issued = ca.issue(&spec, [41; 32]).unwrap();
        let reader = crate::cloud::RunnerSshCertificates;
        let facts: Facts = reader.read(issued.as_bytes(), &ca.public_key()).unwrap();
        assert_eq!(facts.subject_key, [40; 32]);
        assert_eq!(facts.principals, spec.principals);
        assert_eq!(
            facts.valid_before.as_millis(),
            1_700_000_000_000 + 3_600_000
        );
        let other = UserCa::from_seed([22; 32]).public_key();
        assert!(reader.read(issued.as_bytes(), &other).is_err());
    }
}
