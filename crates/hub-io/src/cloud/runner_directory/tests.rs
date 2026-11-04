//! Real runner keys and the contract's own `open_summary`.
use super::{mapping::state, *};
use nocterm_cloud_model::{
    authority::{RunBinding, StreamBinding},
    engine::EnginePrompt,
    identity::*,
    inference::ModelSelection,
    journal::*,
};
use nocterm_hub_model::cloud::summary::ChatState;
use nocterm_keys::{
    AccountKeyPair,
    cloud::{SummaryPins, open_summary},
};
type TestResult = Result<(), Box<dyn std::error::Error>>;

struct FixedHeads(CurrentChatHead);
impl CurrentCloudHeads for FixedHeads {
    fn catalog(
        &self,
        _after: Option<CurrentHeadsCursor>,
        _limit: u16,
    ) -> JournalFuture<CurrentHeadsPage> {
        Box::pin(async { Err(JournalError::NotFound) })
    }
    fn chat(&self, stream: StreamBinding) -> JournalFuture<Option<CurrentChatHead>> {
        let head = (self.0.stream() == &stream).then(|| self.0.clone());
        Box::pin(async move { Ok(head) })
    }
    fn receipt(
        &self,
        _query: OriginalCommandReceiptQuery,
    ) -> JournalFuture<OriginalCommandReceiptObservation> {
        Box::pin(async { Ok(OriginalCommandReceiptObservation::Unknown) })
    }
}

struct Fixture {
    account: AccountKeyPair,
    identity: Arc<CloudRunnerIdentity>,
    stream: StreamBinding,
    _directory: tempfile::TempDir,
}
impl Fixture {
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
        Ok(Self {
            account,
            identity,
            stream: binding.stream(CloudChatId::from_bytes([7; 16])),
            _directory: directory,
        })
    }
    fn head(&self) -> Result<CurrentChatHead, JournalError> {
        let position = HeadRecordPosition::new(
            WriterEpoch::new(1)?,
            EventSequence::from_u64(5),
            JournalSequence::from_u64(9),
            StreamRevision::from_u64(4),
        )?;
        CurrentChatHead::new(
            self.stream.clone(),
            ChatKeyEpoch::new(2)?,
            CurrentHeadFacts {
                writer: Some(WriterEpoch::new(1)?),
                control: ChatControlRevision::from_u64(3),
                deleted: false,
                head: Some(HeadAnchor::new(
                    HeadAnchorKind::Event,
                    position,
                    EventId::from_bytes([8; 16]),
                    None,
                    Fingerprint::from_bytes([9; 32]),
                )),
                event_floor: EventSequence::from_u64(1),
                snapshot: None,
            },
        )
    }
    fn sealer(&self, host: RunnerChatHost) -> Result<SummarySealer, Box<dyn std::error::Error>> {
        Ok(SummarySealer::new(
            self.identity.clone(),
            self.account.public_key().to_bytes(),
            Arc::new(FixedHeads(self.head()?)),
            host,
        )?)
    }
    fn draft(&self, prompt: &str) -> Result<ChatSummaryDraft, Box<dyn std::error::Error>> {
        let run = RunBinding::new(
            self.stream.scope().clone(),
            self.stream.runner(),
            self.stream.runner_epoch(),
            self.stream.chat(),
            RunId::from_bytes([1; 16]),
            RunEpoch::new(1)?,
        );
        let mut facts = ChatSummaryFacts::default();
        facts.observe(
            SummaryObservation::Prompt {
                run: &run,
                prompt: &EnginePrompt::new(vec![prompt.into()])?,
                model: &ModelSelection::new(
                    NamespaceId::new("provider")?,
                    NamespaceId::new("model")?,
                    NamespaceId::new("pricing")?,
                ),
            },
            Timestamp::from_millis(1_000),
        );
        let tracker = ChatSummaryTracker::restore(self.stream.clone(), facts, None)?;
        match tracker.next(Timestamp::from_millis(2_000)) {
            SummaryStep::Seal(draft) => Ok(draft),
            _ => Err("a first revision is due".into()),
        }
    }
    fn pins(&self) -> Result<SummaryPins, Box<dyn std::error::Error>> {
        let public = self.identity.public_identity()?;
        Ok(SummaryPins {
            runner_encryption: *public.encryption_key(),
            runner_signing: *public.signing_key(),
        })
    }
}

#[tokio::test]
async fn a_sealed_summary_opens_with_the_account_key_under_its_runner_pins() -> TestResult {
    let fixture = Fixture::new()?;
    let sealer = fixture.sealer(RunnerChatHost::own())?;
    let sealed = sealer
        .seal(fixture.draft("\n  Fix the build\tnow  \nrest")?)
        .await?;
    assert_eq!((sealed.revision(), sealed.key_epoch().get()), (1, 2));
    let binding = fixture.identity.binding();
    let context = SummaryContext::new(
        ChatCoordinate::new(
            metadata_crypto::namespace(binding)?,
            metadata_crypto::runner(binding),
            [7; 16],
            2,
        )?,
        1,
    )?;
    let opened = open_summary(
        &fixture.account,
        &fixture.pins()?,
        &context,
        sealed.ciphertext().as_bytes(),
    )?;
    let summary = opened.summary();
    assert_eq!(summary.title, "Fix the build now");
    assert_eq!(summary.state, ChatState::Running);
    assert_eq!(summary.host, SummaryHost::Own);
    assert_eq!(summary.features, SUMMARY_FEATURE_PROVIDER_LEASE);
    assert_eq!(summary.control_revision, 3);
    assert_eq!(summary.head.map(|head| head.sequence), Some(5));
    assert_eq!(
        summary.head.map(|head| head.ciphertext_digest),
        Some([9; 32])
    );
    assert_eq!(
        summary.model.as_ref().map(|model| model.model.as_str()),
        Some("model")
    );
    assert_eq!(
        (summary.created_at_ms, summary.last_activity_ms),
        (1_000, 1_000)
    );
    assert!(summary.servers.is_empty());
    let other = AccountKeyPair::generate()?;
    assert!(
        open_summary(
            &other,
            &fixture.pins()?,
            &context,
            sealed.ciphertext().as_bytes()
        )
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn a_managed_seat_names_its_agent_and_takes_no_provider_lease() -> TestResult {
    let fixture = Fixture::new()?;
    let sealer = fixture.sealer(RunnerChatHost::managed("agent-1")?)?;
    let sealed = sealer.seal(fixture.draft("hello")?).await?;
    let binding = fixture.identity.binding();
    let context = SummaryContext::new(
        ChatCoordinate::new(
            metadata_crypto::namespace(binding)?,
            metadata_crypto::runner(binding),
            [7; 16],
            2,
        )?,
        1,
    )?;
    let opened = open_summary(
        &fixture.account,
        &fixture.pins()?,
        &context,
        sealed.ciphertext().as_bytes(),
    )?;
    assert_eq!(
        opened.summary().host,
        SummaryHost::Managed {
            agent_id: CloudTextId::new("agent-1")?
        }
    );
    assert_eq!(opened.summary().features, 0);
    assert!(matches!(
        RunnerChatHost::managed(""),
        Err(RunnerDirectoryError::AgentId(_))
    ));
    Ok(())
}

#[test]
fn a_sealer_for_another_account_key_is_refused() -> TestResult {
    let fixture = Fixture::new()?;
    let result = SummarySealer::new(
        fixture.identity.clone(),
        AccountKeyPair::generate()?.public_key().to_bytes(),
        Arc::new(FixedHeads(fixture.head()?)),
        RunnerChatHost::own(),
    );
    assert!(matches!(result, Err(RunnerDirectoryError::Binding)));
    Ok(())
}

#[test]
fn every_chat_activity_names_its_contract_state() {
    for (activity, expected) in [
        (ChatActivity::Idle, ChatState::Idle),
        (ChatActivity::Running, ChatState::Running),
        (ChatActivity::WaitingApproval, ChatState::WaitingApproval),
        (ChatActivity::NeedsGrant, ChatState::NeedsGrant),
        (ChatActivity::NeedsProvider, ChatState::NeedsProvider),
        (ChatActivity::NeedsSsh, ChatState::NeedsSsh),
        (ChatActivity::Failed, ChatState::Failed),
        (ChatActivity::Deleted, ChatState::Deleted),
    ] {
        assert_eq!(state(activity), expected);
    }
}
