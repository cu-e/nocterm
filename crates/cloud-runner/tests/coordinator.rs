//! Real SQLite admission/engine vertical slices. Injected test codecs are not crypto.
#[path = "coordinator/approval_policy.rs"]
mod approval_policy;
#[path = "coordinator/authentication.rs"]
mod authentication;
#[path = "coordinator/intake.rs"]
mod intake;
#[path = "coordinator/journal_publication.rs"]
mod journal_publication;
#[path = "coordinator/model_completion.rs"]
mod model_completion;
#[path = "coordinator/provider_wake.rs"]
mod provider_wake;
#[path = "coordinator/reconciliation.rs"]
mod reconciliation;
#[path = "coordinator/snapshots.rs"]
mod snapshots;
#[path = "coordinator/supervision.rs"]
mod supervision;
mod support;
use nocterm_ai::model::session::StopReason;
use nocterm_cloud_model::{
    admission::AdmissionLimits, authority::*, coordinator::*, engine::*, grant::*, identity::*,
    inference::*, journal::*, testing::*,
};
use nocterm_cloud_runner::coordinator::{
    CoordinatorExecution, CoordinatorServices, CoordinatorStorage, RunCoordinator,
    ToolApprovalPolicy,
};
use std::sync::Arc;
use support::*;

/// Records the deleted chats the coordinator announced.
#[derive(Default)]
struct RecordedRetirements(std::sync::Mutex<Vec<ChatRetirementRequest>>);
impl ChatRetirement for RecordedRetirements {
    fn retire(&self, request: ChatRetirementRequest) {
        self.0.lock().unwrap().push(request);
    }
}
impl RecordedRetirements {
    fn requests(&self) -> Vec<ChatRetirementRequest> {
        self.0.lock().unwrap().clone()
    }
}

struct Fixture {
    retirements: Arc<RecordedRetirements>,
    root: tempfile::TempDir,
    journal: Arc<nocterm_cloud_runner::DurableRunJournal>,
    actor: RunCoordinator,
    source: Arc<ScriptedCommandSource>,
    engine: Arc<ScriptedEngineFactory>,
    providers: Arc<ScriptedCoordinatorProviders>,
    events: Arc<ObservedCoordinatorEvents>,
    timer: Arc<ManualToolTimer>,
    identity: Arc<ScriptedCoordinatorIdentity>,
    tools: Arc<ScriptedRunTools>,
    storage: CoordinatorStorage,
    cipher: Arc<TestCipher>,
    task_limits: SupervisorLimits,
    receive_budget: CommandReceiveBudget,
}
impl Fixture {
    fn new() -> Self {
        Self::with_task_limits(SupervisorLimits::default())
    }
    fn with_task_limits(task_limits: SupervisorLimits) -> Self {
        Self::with_limits(
            task_limits,
            CommandReceiveBudget::new(IntakeLimits::default()),
        )
    }
    fn with_receive_budget(budget: CommandReceiveBudget) -> Self {
        Self::with_limits(SupervisorLimits::default(), budget)
    }
    fn with_limits(task_limits: SupervisorLimits, receive_budget: CommandReceiveBudget) -> Self {
        Self::with_transport(
            task_limits,
            receive_budget,
            Arc::new(ScriptedPublicationTransport::default()),
        )
    }
    fn with_transport(
        task_limits: SupervisorLimits,
        receive_budget: CommandReceiveBudget,
        transport: Arc<ScriptedPublicationTransport>,
    ) -> Self {
        Self::build(task_limits, receive_budget, transport, None)
    }
    fn with_approval_policy(policy: Arc<dyn ToolApprovalPolicy>) -> Self {
        Self::build(
            SupervisorLimits::default(),
            CommandReceiveBudget::new(IntakeLimits::default()),
            Arc::new(ScriptedPublicationTransport::default()),
            Some(policy),
        )
    }
    fn build(
        task_limits: SupervisorLimits,
        receive_budget: CommandReceiveBudget,
        transport: Arc<ScriptedPublicationTransport>,
        approval: Option<Arc<dyn ToolApprovalPolicy>>,
    ) -> Self {
        let root = private_root();
        let retirements = Arc::new(RecordedRetirements::default());
        let cipher = Arc::new(TestCipher::default());
        let journal = Arc::new(journal(root.path(), cipher.clone()));
        let source = Arc::new(ScriptedCommandSource::new(catalog_binding()));
        let engine = Arc::new(ScriptedEngineFactory::default());
        let events = Arc::new(ObservedCoordinatorEvents::default());
        let timer = Arc::new(ManualToolTimer::default());
        timer.set(1000);
        let selection = selection();
        let pricing =
            Arc::new(ConservativeModelPricing::new(selection.clone(), 4096, 100, 10).unwrap());
        let providers = Arc::new(ScriptedCoordinatorProviders::new(
            selection,
            engine.clone(),
            pricing,
        ));
        let provider_access = providers.clone();
        let storage = CoordinatorStorage::with_task_limits(
            catalog_binding(),
            journal.clone(),
            Arc::new(journal.cleanup()),
            events.clone(),
            nocterm_cloud_runner::CoordinatorPublicationBudgets::new(
                receive_budget.clone(),
                IntakeLimits::default().pool(IntakeLane::Ordinary),
            ),
            tokio::runtime::Handle::current(),
            task_limits,
        )
        .with_publication(publication_config(&journal, transport));
        let tools = Arc::new(ScriptedRunTools::default());
        let mut execution = CoordinatorExecution::new(
            providers,
            Arc::new(RefusedSessionConsent),
            tools.clone(),
            Arc::new(FixtureArgumentFingerprint),
        )
        .with_retirement(retirements.clone());
        if let Some(policy) = approval {
            execution = execution.with_approval_policy(policy);
        }
        let identity = Arc::new(ScriptedCoordinatorIdentity::default());
        let services = CoordinatorServices::new(
            source.clone(),
            storage.clone(),
            execution,
            identity.clone(),
            timer.clone(),
        )
        .unwrap();
        let admissions = AdmissionLimits::new(
            task_limits.workers(),
            task_limits.workers().min(2),
            128,
            32,
            8,
            64,
        )
        .unwrap();
        let actor =
            RunCoordinator::with_receive_budget(services, admissions, receive_budget.clone())
                .unwrap();
        Self {
            retirements,
            root,
            journal,
            actor,
            source,
            engine,
            providers: provider_access,
            events,
            timer,
            identity,
            tools,
            storage,
            cipher,
            task_limits,
            receive_budget,
        }
    }
    fn restart(&mut self) {
        self.actor.fence();
        self.cipher = Arc::new(TestCipher::default());
        self.journal = Arc::new(journal(self.root.path(), self.cipher.clone()));
        let pricing = Arc::new(ConservativeModelPricing::new(selection(), 4096, 100, 10).unwrap());
        let providers = Arc::new(ScriptedCoordinatorProviders::new(
            selection(),
            self.engine.clone(),
            pricing,
        ));
        self.providers = providers.clone();
        let storage = CoordinatorStorage::with_task_limits(
            catalog_binding(),
            self.journal.clone(),
            Arc::new(self.journal.cleanup()),
            self.events.clone(),
            nocterm_cloud_runner::CoordinatorPublicationBudgets::new(
                self.receive_budget.clone(),
                IntakeLimits::default().pool(IntakeLane::Ordinary),
            ),
            tokio::runtime::Handle::current(),
            self.task_limits,
        )
        .with_publication(publication_config(
            &self.journal,
            Arc::new(ScriptedPublicationTransport::default()),
        ));
        let execution = CoordinatorExecution::new(
            providers,
            Arc::new(RefusedSessionConsent),
            self.tools.clone(),
            Arc::new(FixtureArgumentFingerprint),
        )
        .with_retirement(self.retirements.clone());
        self.storage = storage.clone();
        let services = CoordinatorServices::new(
            self.source.clone(),
            storage,
            execution,
            self.identity.clone(),
            self.timer.clone(),
        )
        .unwrap();
        let admissions = AdmissionLimits::new(
            self.task_limits.workers(),
            self.task_limits.workers().min(2),
            128,
            32,
            8,
            64,
        )
        .unwrap();
        self.actor =
            RunCoordinator::with_receive_budget(services, admissions, self.receive_budget.clone())
                .unwrap();
    }
    async fn binding(&self) -> RunBinding {
        let page = self
            .journal
            .read(stream(), JournalSequence::default(), limits())
            .await
            .unwrap();
        page.records()
            .iter()
            .find_map(|stored| match stored.record().kind() {
                JournalRecordKind::CommandAccepted {
                    kind: CommandKind::SubmitPrompt,
                    ..
                } => stored.record().context().run().cloned(),
                _ => None,
            })
            .unwrap()
    }
    async fn control(&self) -> ChatControlRevision {
        self.journal
            .head(stream())
            .await
            .unwrap()
            .unwrap()
            .position()
            .control()
    }
}
fn issuer() -> CommandIssuer {
    CommandIssuer::new(NamespaceId::new("device").unwrap(), fp(51), fp(52))
}
fn window() -> GrantWindow {
    GrantWindow::new(Timestamp::from_millis(1000), Timestamp::from_millis(61000)).unwrap()
}
fn selection() -> ModelSelection {
    ModelSelection::new(
        NamespaceId::new("provider").unwrap(),
        NamespaceId::new("model").unwrap(),
        NamespaceId::new("fixed-v1").unwrap(),
    )
}
fn budget() -> ModelBudget {
    ModelBudget::new(2, 4096, 4096, 100).unwrap()
}
fn command(
    id: u8,
    control: ChatControlRevision,
    run: Option<RunBinding>,
    body: CommandBody,
) -> ReceivedCommand {
    ReceivedCommand::new(
        stream(),
        CommandAcceptance::new(
            CommandId::from_bytes([id; 16]),
            body.kind(),
            fp(id),
            issuer(),
            control,
        ),
        window(),
        run,
        body,
        None,
    )
    .unwrap()
}
fn submit(id: u8, control: ChatControlRevision) -> ReceivedCommand {
    let target = TargetBinding::new(fp(21), fp(22), NamespaceId::new("user").unwrap(), fp(23));
    let auth = AuthenticationBinding::new(fp(24), fp(25));
    command(
        id,
        control,
        None,
        CommandBody::Submit(PromptRequest::new(
            EnginePrompt::new(vec!["original prompt".into()]).unwrap(),
            selection(),
            RunDestination::new(target, auth, None).unwrap(),
            PromptAllowance::new(budget(), 20, window()).unwrap(),
        )),
    )
}
fn inference(id: u8, control: ChatControlRevision, run: RunBinding) -> ReceivedCommand {
    let grant = InferenceGrant::new(
        GrantId::from_bytes([id; 16]),
        GrantEpoch::new(1).unwrap(),
        GrantOrigin::new(
            issuer(),
            CommandId::from_bytes([id; 16]),
            control.next().unwrap(),
        ),
        InferenceTask::new(run.clone(), selection(), budget(), 20, window()).unwrap(),
    );
    command(id, control, Some(run), CommandBody::InferenceGrant(grant))
}
fn complete() -> ScriptedEngineTurn {
    let usage = UsageObservation {
        output_tokens: Some(2),
        cost_micros: Some(10),
        ..UsageObservation::default()
    };
    ScriptedEngineTurn {
        request_bytes: 100,
        output_cap: 20,
        events: vec![EngineEvent::Text(
            EngineText::new(TextChannel::Assistant, "answer".into()).unwrap(),
        )],
        observation: ModelCallObservation::new(usage, 6).unwrap(),
        action: EngineAction::Complete(EngineCompletion {
            stop: StopReason::EndTurn,
            usage,
        }),
    }
}
#[tokio::test]
async fn accepted_prompt_and_budget_are_durable_before_model_completion() {
    let fixture = Fixture::new();
    fixture.engine.push(complete());
    let prompt = submit(1, ChatControlRevision::default());
    let receipt = fixture.actor.accept(prompt.clone()).await.unwrap();
    assert_eq!(receipt.position().records().get(), 2);
    assert_eq!(fixture.engine.calls(), 0);
    assert!(!fixture.actor.drive_one().await.unwrap());
    let binding = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, binding.clone()))
        .await
        .unwrap();
    assert!(fixture.actor.drive_one().await.unwrap());
    assert_eq!(fixture.engine.calls(), 1);
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    let kinds = page
        .records()
        .iter()
        .map(|stored| stored.record().kind())
        .collect::<Vec<_>>();
    let reserved = kinds
        .iter()
        .position(|kind| matches!(kind, JournalRecordKind::BudgetReserved { .. }))
        .unwrap();
    let action = kinds
        .iter()
        .position(|kind| matches!(kind, JournalRecordKind::ModelAction { .. }))
        .unwrap();
    assert!(reserved < action);
    assert!(
        fixture
            .events
            .events()
            .iter()
            .any(|event| matches!(event,CoordinatorEvent::PromptAccepted{run,..} if run==&binding))
    );
    let duplicate = fixture.actor.accept(prompt).await.unwrap();
    assert_eq!(duplicate.position(), receipt.position());
    assert_eq!(fixture.engine.calls(), 1);
}
#[tokio::test]
async fn stop_fences_queued_run_and_conflicting_same_id_cannot_replace_it() {
    let fixture = Fixture::new();
    fixture.engine.push(complete());
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let binding = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, binding.clone()))
        .await
        .unwrap();
    let stop = command(
        3,
        fixture.control().await,
        Some(binding.clone()),
        CommandBody::Stop,
    );
    let receipt = fixture.actor.accept(stop.clone()).await.unwrap();
    assert!(!fixture.actor.drive_one().await.unwrap());
    assert_eq!(fixture.engine.calls(), 0);
    let altered = command(
        3,
        stop.acceptance().expected_control(),
        Some(binding),
        CommandBody::RevokeGrant(GrantReference::new(
            GrantClass::Inference,
            GrantId::from_bytes([2; 16]),
            GrantEpoch::new(1).unwrap(),
        )),
    );
    assert!(matches!(
        fixture.actor.accept(altered).await,
        Err(CoordinatorError::Journal(JournalError::Conflict))
    ));
    assert_eq!(
        fixture.actor.accept(stop).await.unwrap().position(),
        receipt.position()
    );
}
#[tokio::test]
async fn revoked_source_never_creates_a_receipt_or_engine() {
    let fixture = Fixture::new();
    fixture.source.revoke();
    assert!(matches!(
        fixture
            .actor
            .accept(submit(1, ChatControlRevision::default()))
            .await,
        Err(CoordinatorError::StaleSource)
    ));
    assert_eq!(fixture.engine.calls(), 0);
    assert!(fixture.journal.head(stream()).await.unwrap().is_none());
}

#[tokio::test]
async fn durable_queued_work_recovers_once_and_completed_work_never_reissues_model_io() {
    let mut fixture = Fixture::new();
    fixture.engine.push(complete());
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let binding = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, binding.clone()))
        .await
        .unwrap();
    fixture.restart();
    fixture.actor.recover().await.unwrap();
    assert!(fixture.actor.drive_one().await.unwrap());
    assert_eq!(fixture.engine.calls(), 1);
    fixture.restart();
    fixture.actor.recover().await.unwrap();
    assert!(!fixture.actor.drive_one().await.unwrap());
    assert_eq!(fixture.engine.calls(), 1);
    assert_eq!(
        fixture.actor.status(&binding).await.unwrap(),
        RunStatus::Completed
    );
}
#[tokio::test]
async fn lost_worker_after_durable_model_reservation_never_replays_that_http_attempt() {
    let mut fixture = Fixture::new();
    fixture.engine.push(complete());
    let (entered, _release) = fixture.engine.pause_after_reserve();
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let binding = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, binding.clone()))
        .await
        .unwrap();
    let actor = fixture.actor.clone();
    let worker = tokio::spawn(async move { actor.drive_one().await });
    entered.wait().await;
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    fixture.restart();
    fixture.actor.recover().await.unwrap();
    assert!(!fixture.actor.drive_one().await.unwrap());
    assert_eq!(fixture.engine.calls(), 0);
    assert_eq!(
        fixture.actor.status(&binding).await.unwrap(),
        RunStatus::Interrupted
    );
}

#[tokio::test]
async fn stopping_an_old_original_run_does_not_retarget_the_new_run() {
    let fixture = Fixture::new();
    fixture.engine.push(complete());
    fixture.engine.push(complete());
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let first = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, first.clone()))
        .await
        .unwrap();
    fixture.actor.drive_one().await.unwrap();
    fixture
        .actor
        .accept(submit(3, fixture.control().await))
        .await
        .unwrap();
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    let second = page
        .records()
        .iter()
        .find_map(|stored| match stored.record().kind() {
            JournalRecordKind::CommandAccepted {
                id,
                kind: CommandKind::SubmitPrompt,
            } if *id == CommandId::from_bytes([3; 16]) => stored.record().context().run().cloned(),
            _ => None,
        })
        .unwrap();
    fixture
        .actor
        .accept(inference(4, fixture.control().await, second.clone()))
        .await
        .unwrap();
    fixture
        .actor
        .accept(command(
            5,
            fixture.control().await,
            Some(first),
            CommandBody::Stop,
        ))
        .await
        .unwrap();
    assert!(fixture.actor.drive_one().await.unwrap());
    assert_eq!(fixture.engine.calls(), 2);
    assert_eq!(
        fixture.actor.status(&second).await.unwrap(),
        RunStatus::Completed
    );
}

#[tokio::test]
async fn a_new_finite_grant_wakes_the_same_waiting_prompt_without_repeating_http() {
    let fixture = Fixture::new();
    fixture.engine.push(complete());
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let run = fixture.binding().await;
    let control = fixture.control().await;
    let grant = InferenceGrant::new(
        GrantId::from_bytes([2; 16]),
        GrantEpoch::new(1).unwrap(),
        GrantOrigin::new(
            issuer(),
            CommandId::from_bytes([2; 16]),
            control.next().unwrap(),
        ),
        InferenceTask::new(
            run.clone(),
            selection(),
            ModelBudget::new(1, 1, 1, 1).unwrap(),
            1,
            window(),
        )
        .unwrap(),
    );
    fixture
        .actor
        .accept(command(
            2,
            control,
            Some(run.clone()),
            CommandBody::InferenceGrant(grant),
        ))
        .await
        .unwrap();
    assert!(fixture.actor.drive_one().await.is_err());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::NeedsGrant
    );
    assert_eq!(fixture.engine.calls(), 0);
    assert!(!fixture.actor.drive_one().await.unwrap());
    fixture.engine.push(complete());
    fixture
        .actor
        .accept(inference(3, fixture.control().await, run.clone()))
        .await
        .unwrap();
    assert!(fixture.actor.drive_one().await.unwrap());
    assert_eq!(
        fixture.actor.status(&run).await.unwrap(),
        RunStatus::Completed
    );
    assert_eq!(fixture.engine.calls(), 1);
    assert_eq!(
        fixture
            .events
            .events()
            .iter()
            .filter(|event| matches!(event, CoordinatorEvent::PromptAccepted { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn dropping_a_foreground_worker_retires_its_reservation_before_the_next_run() {
    let fixture = Fixture::new();
    fixture.engine.push(complete());
    let (entered, _release) = fixture.engine.pause_after_reserve();
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let original = fixture.binding().await;
    fixture
        .actor
        .accept(inference(2, fixture.control().await, original.clone()))
        .await
        .unwrap();
    let actor = fixture.actor.clone();
    let worker = tokio::spawn(async move { actor.drive_one().await });
    entered.wait().await;
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fixture.actor.status(&original).await.unwrap() != RunStatus::Interrupted {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.engine.push(complete());
    let receipt = fixture
        .actor
        .accept(submit(3, fixture.control().await))
        .await
        .unwrap();
    let page = fixture
        .journal
        .read(stream(), JournalSequence::default(), limits())
        .await
        .unwrap();
    let next = page
        .records()
        .iter()
        .find_map(|stored| match stored.record().kind() {
            JournalRecordKind::CommandAccepted {
                id,
                kind: CommandKind::SubmitPrompt,
            } if *id == receipt.acceptance().id() => stored.record().context().run().cloned(),
            _ => None,
        })
        .unwrap();
    fixture
        .actor
        .accept(inference(4, fixture.control().await, next.clone()))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !fixture.actor.drive_one().await.unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.actor.status(&next).await.unwrap(),
        RunStatus::Completed
    );
    assert_eq!(fixture.engine.calls(), 1);
    assert_eq!(
        fixture.actor.status(&original).await.unwrap(),
        RunStatus::Interrupted
    );
}

#[tokio::test]
async fn recovery_keeps_deleted_catalog_entries_as_cleanup_only_and_continues_other_chats() {
    let mut fixture = Fixture::new();
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    let deletion = command(2, fixture.control().await, None, CommandBody::DeleteChat);
    let receipt = fixture.actor.accept(deletion.clone()).await.unwrap();
    let original = stream();
    let another = StreamBinding::new(
        original.scope().clone(),
        original.runner(),
        original.runner_epoch(),
        CloudChatId::from_bytes([0x77; 16]),
    );
    let prompt = submit(3, ChatControlRevision::default());
    let other = ReceivedCommand::new(
        another.clone(),
        prompt.acceptance().clone(),
        window(),
        None,
        prompt.body().clone(),
        None,
    )
    .unwrap();
    fixture.actor.accept(other).await.unwrap();
    fixture.restart();
    fixture.actor.recover().await.unwrap();
    let head = fixture.journal.head(stream()).await.unwrap().unwrap();
    assert!(head.deleted());
    assert!(fixture.journal.head(another).await.unwrap().is_some());
    assert_eq!(
        fixture.actor.accept(deletion).await.unwrap().position(),
        receipt.position()
    );
    assert!(matches!(
        fixture
            .actor
            .accept(submit(4, head.position().control()))
            .await,
        Err(CoordinatorError::Stopped)
    ));
    fixture.actor.recover().await.unwrap();
    assert_eq!(
        fixture
            .journal
            .head(stream())
            .await
            .unwrap()
            .unwrap()
            .lease(),
        head.lease()
    );
}

#[tokio::test]
async fn a_durable_deletion_is_announced_with_the_requesting_device() {
    let fixture = Fixture::new();
    fixture
        .actor
        .accept(submit(1, ChatControlRevision::default()))
        .await
        .unwrap();
    assert!(fixture.retirements.requests().is_empty());
    let deletion = command(2, fixture.control().await, None, CommandBody::DeleteChat);
    fixture.actor.accept(deletion).await.unwrap();
    assert_eq!(
        fixture.retirements.requests(),
        vec![ChatRetirementRequest::new(
            stream(),
            NamespaceId::new("device").unwrap()
        )]
    );
}
