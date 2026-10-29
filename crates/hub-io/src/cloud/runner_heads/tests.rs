//! Real signed/boxed native facts and actual finite worker ownership.
use super::*;
use mapping::{HeadBinding, HeadReadValue};
use nocterm_cloud_model::{
    authority::RunBinding, coordinator::IntakePoolLimits, identity::*, journal::*,
};
use nocterm_hub_model::cloud::{
    CloudTextId, CommandIssuer,
    head::{CatalogHeadCursor, HeadQuery, HeadTarget, HeadValue},
};
use nocterm_keys::{AccountKeyPair, DeviceKey, cloud::open_head_witness};
use nocterm_session::testing::Gate;
use std::sync::{Arc, Weak};
use tokio::sync::{Semaphore, oneshot};
type TestResult = Result<(), Box<dyn std::error::Error>>;
struct HeadFixture {
    binding: JournalCatalogBinding,
    mapped: HeadBinding,
    identity: Arc<CloudRunnerIdentity>,
    account: AccountKeyPair,
    device: DeviceKey,
    _directory: tempfile::TempDir,
}
impl HeadFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let account = AccountKeyPair::generate()?;
        let seed = crate::cloud::runner_test_support::binding(2);
        let binding = JournalCatalogBinding::new(
            AccountScope::new(
                seed.scope().hub().clone(),
                AccountIdentity::new(
                    seed.scope().account().id().clone(),
                    Fingerprint::from_bytes(Sha256::digest(account.public_key().as_bytes()).into()),
                ),
            ),
            seed.runner(),
            seed.epoch(),
        );
        let identity = Arc::new(CloudRunnerIdentity::create_new(
            &directory.path().join("identity"),
            binding.clone(),
        )?);
        let public = identity.public_identity()?;
        let scope = HeadProbeScope {
            namespace: crate::cloud::metadata_crypto::namespace(&binding)?,
            runner: crate::cloud::metadata_crypto::runner(&binding),
            keys: HeadProbeKeys {
                account: account.public_key().to_bytes(),
                runner_encryption: *public.encryption_key(),
                runner_signing: *public.signing_key(),
            },
        };
        let mapped = HeadBinding::new(binding.clone(), scope)?;
        Ok(Self {
            binding,
            mapped,
            identity,
            account,
            device: DeviceKey::generate()?,
            _directory: directory,
        })
    }
    fn query(&self, target: HeadTarget) -> Result<HeadQuery, Box<dyn std::error::Error>> {
        Ok(HeadQuery {
            scope: self.mapped.scope().clone(),
            issuer: CommandIssuer {
                device_id: CloudTextId::new("device")?,
                signing_key: self.device.public_key(),
                access_token_digest: [3; 32],
            },
            probe_id: nocterm_keys::random_bytes::<16>()?,
            challenge: nocterm_keys::random_bytes::<32>()?,
            window: CapabilityWindow {
                issued_at_ms: 100_000,
                expires_at_ms: 110_000,
            },
            target,
        })
    }
    fn head(&self, id: u8, deleted: bool) -> Result<CurrentChatHead, Box<dyn std::error::Error>> {
        let stream = self.binding.stream(CloudChatId::from_bytes([id; 16]));
        let run = RunBinding::new(
            stream.scope().clone(),
            stream.runner(),
            stream.runner_epoch(),
            stream.chat(),
            RunId::from_bytes([7; 16]),
            RunEpoch::new(2)?,
        );
        let head = HeadAnchor::new(
            HeadAnchorKind::Event,
            HeadRecordPosition::new(
                WriterEpoch::new(2)?,
                EventSequence::from_u64(5),
                JournalSequence::from_u64(7),
                StreamRevision::from_u64(9),
            )?,
            EventId::from_bytes([6; 16]),
            Some(run),
            Fingerprint::from_bytes([8; 32]),
        );
        Ok(CurrentChatHead::new(
            stream,
            ChatKeyEpoch::new(11)?,
            CurrentHeadFacts {
                writer: Some(WriterEpoch::new(4)?),
                control: ChatControlRevision::from_u64(12),
                deleted,
                head: Some(head),
                event_floor: EventSequence::from_u64(if deleted { 6 } else { 1 }),
                snapshot: None,
            },
        )?)
    }
    fn roundtrip(
        &self,
        query: &HeadQuery,
        value: &HeadReadValue,
    ) -> Result<nocterm_hub_model::cloud::head::SignedHeadWitness, Box<dyn std::error::Error>> {
        // Authenticate the real device/account query before using its original
        // challenge, then sign only the native-to-public descriptive mapping.
        let signed = nocterm_hub_model::cloud::head::SignedHeadQuery::new(
            query.clone(),
            self.device.sign(&query.canonical_bytes()?),
        )?;
        let encrypted = nocterm_keys::cloud::seal_head_query(&self.account, &signed)?;
        let opened = self.identity.with_keys(|keys| {
            nocterm_keys::cloud::open_head_query(
                keys.encryption(),
                self.mapped.scope(),
                query.probe_id,
                &encrypted,
                100_001,
            )
        })??;
        let value = self.mapped.value(opened.query(), value)?;
        let witness = self.identity.with_keys(|keys| {
            keys.signing().attest_head(HeadWitness {
                query: opened.query().clone(),
                observed_at_ms: 100_001,
                value,
            })
        })??;
        let sealed = self
            .identity
            .with_keys(|keys| seal_head_witness(keys.encryption(), &witness))??;
        Ok(open_head_witness(&self.account, query, &sealed, 100_001)?)
    }
}

#[test]
fn native_head_witness_keeps_exact_catalog_root_deleted_original_writer_and_epoch() -> TestResult {
    let fixture = HeadFixture::new()?;
    let query = fixture.query(HeadTarget::Catalog { after: None })?;
    let page = CurrentHeadsPage::new(
        fixture.binding.clone(),
        17,
        Fingerprint::from_bytes([9; 32]),
        2,
        None,
        vec![fixture.head(5, true)?],
        true,
    )?;
    let verified = fixture.roundtrip(&query, &HeadReadValue::Catalog(page))?;
    let HeadValue::Catalog(page) = &verified.witness().value else {
        return Err("catalog".into());
    };
    assert_eq!(page.chats[0].current_writer, Some(4));
    assert_eq!(
        page.chats[0]
            .head
            .as_ref()
            .ok_or("head")?
            .position
            .writer_epoch,
        2
    );
    assert_eq!(page.chats[0].chat_key_epoch, 11);
    assert!(page.chats[0].deleted);
    let cursor = page.next_cursor().ok_or("cursor")?;
    assert_eq!(
        cursor,
        CatalogHeadCursor {
            revision: 17,
            digest: [9; 32],
            after: [5; 16],
            seen: 1
        }
    );
    let mut moved = query.clone();
    moved.challenge[0] ^= 1;
    assert!(verified.require_query(&moved, 100_001).is_err());
    let missing = fixture.query(HeadTarget::Chat {
        chat_id: [5; 16],
        chat_key_epoch: 11,
    })?;
    assert!(matches!(
        fixture.mapped.value(&missing, &HeadReadValue::Chat(None)),
        Err(RunnerHeadError::MissingChat)
    ));
    let wrong = fixture.query(HeadTarget::Chat {
        chat_id: [5; 16],
        chat_key_epoch: 12,
    })?;
    assert!(matches!(
        fixture
            .mapped
            .value(&wrong, &HeadReadValue::Chat(Some(fixture.head(5, true)?))),
        Err(RunnerHeadError::Binding)
    ));
    let mut clock = ClockFence::default();
    clock.observe(100_000)?;
    assert!(matches!(
        clock.observe(99_999),
        Err(RunnerHeadError::Expired)
    ));
    assert!(matches!(
        clock.observe(100_001),
        Err(RunnerHeadError::Expired)
    ));
    fixture.identity.retire();
    assert!(
        fixture
            .roundtrip(&query, &HeadReadValue::Chat(None))
            .is_err()
    );
    Ok(())
}

fn reservation(work: &RunnerHeadWork) -> Result<Arc<work::HeadReservation>, RunnerHeadError> {
    work.reserve(
        Arc::new(Semaphore::new(1))
            .try_acquire_owned()
            .map_err(|_| JournalError::Busy)?,
    )
}
async fn dropped<T>(weak: &Weak<T>) -> TestResult {
    tokio::time::timeout(Duration::from_secs(5), async {
        while weak.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    Ok(())
}
#[tokio::test]
async fn actual_head_read_cancellation_and_unclaimed_results_keep_slot_and_quota() -> TestResult {
    let bytes = RunnerHeadWork::operation_bytes();
    let budget = Arc::new(PublicationBudget::maintenance(IntakePoolLimits::new(
        2 * bytes,
        2,
        2,
    )?));
    let work =
        RunnerHeadWork::with_max_reads(budget.clone(), tokio::runtime::Handle::current(), 1)?;
    let first = reservation(&work)?;
    let weak = Arc::downgrade(&first);
    let second = reservation(&work)?;
    let gate = Gate::pending();
    let blocked = gate.clone();
    let (started, ready) = oneshot::channel();
    let task = work.spawn_read(first.clone(), move || {
        let _ = started.send(());
        futures::executor::block_on(blocked.wait());
        Ok(vec![1; 32])
    })?;
    ready.await?;
    drop(task);
    drop(first);
    assert!(weak.upgrade().is_some());
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(bytes, 1)?)
            .is_err()
    );
    assert!(matches!(
        work.spawn_read(second.clone(), || Ok(())),
        Err(RunnerHeadError::Journal(JournalError::Busy))
    ));
    gate.release();
    dropped(&weak).await?;
    let output = work
        .spawn_read(second.clone(), || {
            Err::<Vec<u8>, _>(JournalError::Busy.into())
        })?
        .await?;
    assert!(matches!(
        output.value(),
        Err(RunnerHeadError::Journal(JournalError::Busy))
    ));
    let weak_second = Arc::downgrade(&second);
    drop(second);
    let third = reservation(&work)?;
    assert!(matches!(
        work.spawn_read(third.clone(), || Ok(())),
        Err(RunnerHeadError::Journal(JournalError::Busy))
    ));
    assert!(weak_second.upgrade().is_some());
    drop(output);
    dropped(&weak_second).await?;
    let output = work.spawn_read(third.clone(), || Ok(37))?.await?;
    assert_eq!(*output.value()?, 37);
    drop(output);
    drop(third);
    assert!(
        budget
            .try_reserve(PublicationAllocation::new(2 * bytes, 2)?)
            .is_ok()
    );
    Ok(())
}
