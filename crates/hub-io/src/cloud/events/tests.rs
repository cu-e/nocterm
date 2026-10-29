//! Actual SDK interoperability at the native encoder boundary.
use super::*;
use crate::cloud::{CloudRunnerIdentity, RunnerJournalCrypto};
use nocterm_cloud_model::{
    authority::RunBinding,
    coordinator::{IntakePoolLimits, RunStatus},
    engine::{EngineEvent, EngineText, TextChannel, UsageObservation},
    identity::*,
    journal::{JournalCatalogBinding, PublicationBudget},
};
use nocterm_core::clock::{FixedClock, WallTime};
use nocterm_hub_model::cloud::events::{CloudEventPayload, CloudUsage};
use nocterm_keys::{
    AccountKeyPair, PublicKey,
    cloud::{ChatKey, open_event, seal_chat_key_to_runner},
};
use sha2::{Digest, Sha256};

struct Fixture {
    _directory: tempfile::TempDir,
    identity: Arc<CloudRunnerIdentity>,
    material: Arc<RunnerJournalCrypto>,
    key: ChatKey,
    lease: WriterLease,
    run: RunBinding,
}
impl Fixture {
    fn new() -> Self {
        let account = AccountKeyPair::generate().unwrap();
        let binding = JournalCatalogBinding::new(
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
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(4).unwrap(),
        );
        let directory = tempfile::tempdir().unwrap();
        let identity =
            Arc::new(CloudRunnerIdentity::create_new(directory.path(), binding.clone()).unwrap());
        let material = Arc::new(RunnerJournalCrypto::new(identity.clone(), 1).unwrap());
        let stream = binding.stream(CloudChatId::from_bytes([5; 16]));
        let epoch = ChatKeyEpoch::new(6).unwrap();
        let coordinate = nocterm_hub_model::cloud::ChatCoordinate::new(
            crate::cloud::metadata_crypto::namespace(&binding).unwrap(),
            crate::cloud::metadata_crypto::runner(&binding),
            [5; 16],
            epoch.get(),
        )
        .unwrap();
        let key = ChatKey::generate().unwrap();
        let recipient =
            PublicKey::from_bytes(identity.public_identity().unwrap().encryption_key()).unwrap();
        let grant = seal_chat_key_to_runner(&key, &account, &recipient, &coordinate).unwrap();
        material
            .install_grant(&stream, epoch, account.public_key().to_bytes(), &grant)
            .unwrap();
        let run = RunBinding::new(
            stream.scope().clone(),
            stream.runner(),
            stream.runner_epoch(),
            stream.chat(),
            RunId::from_bytes([7; 16]),
            RunEpoch::new(8).unwrap(),
        );
        Self {
            _directory: directory,
            identity,
            material,
            key,
            lease: WriterLease::new(stream, WriterEpoch::new(9).unwrap()),
            run,
        }
    }
    fn authority(&self) -> nocterm_cloud_model::authority::ExecutionAuthority {
        use nocterm_cloud_model::authority::{
            AuthenticationBinding, ExecutionAuthority, SessionReceipt, TargetBinding,
        };
        let target = TargetBinding::new(
            Fingerprint::from_bytes([10; 32]),
            Fingerprint::from_bytes([11; 32]),
            NamespaceId::new("principal").unwrap(),
            Fingerprint::from_bytes([12; 32]),
        );
        ExecutionAuthority::new(
            self.run.clone(),
            SessionReceipt::new(
                RemoteSessionId::from_bytes([13; 16]),
                SessionEpoch::new(1).unwrap(),
                target,
                AuthenticationBinding::new(
                    Fingerprint::from_bytes([14; 32]),
                    Fingerprint::from_bytes([15; 32]),
                ),
            ),
            GrantId::from_bytes([16; 16]),
            GrantEpoch::new(1).unwrap(),
        )
    }
    fn encoder(&self) -> RunnerEventEncoder {
        RunnerEventEncoder::new(
            self.material.clone(),
            Arc::new(FixedClock::new(WallTime {
                unix_seconds: 1_700_000_000,
                utc_offset_seconds: 0,
            })),
        )
    }
    fn budget(bytes: usize) -> PublicationBudget {
        PublicationBudget::maintenance(IntakePoolLimits::new(bytes, 32, 32).unwrap())
    }
}

#[test]
fn runner_events_preserve_original_signed_context_and_complete_unknown_usage() {
    let fixture = Fixture::new();
    let encoder = fixture.encoder();
    let work = Fixture::budget(4 * 1024 * 1024);
    let mut next = JournalPosition::default();
    let events = [
        CoordinatorEvent::Progress {
            run: fixture.run.clone(),
            event: EngineEvent::Text(
                EngineText::new(TextChannel::Thought, "native progress".into()).unwrap(),
            ),
        },
        CoordinatorEvent::Progress {
            run: fixture.run.clone(),
            event: EngineEvent::Usage(UsageObservation {
                input_tokens: Some(7),
                cost_micros: None,
                ..Default::default()
            }),
        },
        CoordinatorEvent::Status {
            run: fixture.run.clone(),
            status: RunStatus::Completed,
        },
    ];
    for (index, event) in events.iter().enumerate() {
        next = next.next(1, 1, index == 0).unwrap();
        let publication =
            CoordinatorEventEncoder::encode(&encoder, &fixture.lease, next, event, &work).unwrap();
        assert_ne!(
            publication.id().as_bytes(),
            publication.delivery_id().as_bytes()
        );
        let context = crate::cloud::runner_transport::record_context(
            publication.stream(),
            publication.key_epoch(),
            publication.anchor(),
        )
        .unwrap();
        let public = fixture.identity.public_identity().unwrap();
        let opened = open_event(
            &fixture.key,
            public.signing_key(),
            &context,
            publication.sealed().as_bytes(),
        )
        .unwrap();
        let record = opened.record().unwrap();
        let decoded = CloudEvent::from_record(&record).unwrap();
        assert_eq!(decoded.timestamp_ms, 1_700_000_000_000);
        assert_eq!(decoded.control_revision, 1);
        match (index, decoded.payload) {
            (
                0,
                CloudEventPayload::Text {
                    step: None,
                    text: "native progress",
                    ..
                },
            ) => {}
            (1, CloudEventPayload::Usage { usage, .. }) => assert_eq!(
                usage,
                CloudUsage {
                    input_tokens: Some(7),
                    ..Default::default()
                }
            ),
            (2, CloudEventPayload::Status { .. }) => {}
            _ => panic!("native observation changed"),
        }
        let mut tampered = publication.sealed().as_bytes().to_vec();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(open_event(&fixture.key, public.signing_key(), &context, &tampered).is_err());
    }
}

#[test]
fn runner_events_refuse_missing_capacity_wrong_stream_and_retired_material() {
    let fixture = Fixture::new();
    let encoder = fixture.encoder();
    let next = JournalPosition::default().next(1, 1, true).unwrap();
    let event = CoordinatorEvent::Status {
        run: fixture.run.clone(),
        status: RunStatus::Running,
    };
    assert!(matches!(
        CoordinatorEventEncoder::encode(
            &encoder,
            &fixture.lease,
            next,
            &event,
            &Fixture::budget(1)
        ),
        Err(JournalError::Capacity)
    ));
    let other = WriterLease::new(
        nocterm_cloud_model::authority::StreamBinding::new(
            fixture.run.scope().clone(),
            fixture.run.runner(),
            fixture.run.runner_epoch(),
            CloudChatId::from_bytes([88; 16]),
        ),
        fixture.lease.epoch(),
    );
    assert!(
        CoordinatorEventEncoder::encode(
            &encoder,
            &other,
            next,
            &event,
            &Fixture::budget(1024 * 1024)
        )
        .is_err()
    );
    fixture.material.retire();
    assert!(
        CoordinatorEventEncoder::encode(
            &encoder,
            &fixture.lease,
            next,
            &event,
            &Fixture::budget(1024 * 1024)
        )
        .is_err()
    );
}

mod tools;

mod lifecycle;

mod snapshot;
