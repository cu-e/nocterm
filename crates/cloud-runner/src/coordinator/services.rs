use super::{io::JournalIo, supervisor::TaskSupervisor};
use nocterm_cloud_model::{
    coordinator::*,
    journal::{JournalCatalogBinding, RunJournalCleanup},
    tool::{ArgumentFingerprint, RunToolRuntimeFactory, ToolTimer},
};
use std::sync::Arc;
use tokio::runtime::Handle;

/// Shared native publication owner. Foreground, retained job and cleanup
/// facades must use this same dispatcher, never independent disk queues.
#[derive(Clone)]
pub struct CoordinatorStorage {
    binding: JournalCatalogBinding,
    io: JournalIo,
    encoder: Arc<dyn CoordinatorEventEncoder>,
    work: crate::CoordinatorPublicationBudgets,
    runs: super::authority::RunAuthorities,
    tasks: TaskSupervisor,
    publication: Option<Arc<super::publication::CoordinatorPublication>>,
    summaries: Option<Arc<super::summary_publication::CoordinatorSummaries>>,
}
impl CoordinatorStorage {
    pub fn new(
        binding: JournalCatalogBinding,
        journal: Arc<dyn CoordinatorJournal>,
        cleanup: Arc<dyn RunJournalCleanup>,
        encoder: Arc<dyn CoordinatorEventEncoder>,
        work: crate::CoordinatorPublicationBudgets,
        executor: Handle,
    ) -> Self {
        Self::with_task_limits(
            binding,
            journal,
            cleanup,
            encoder,
            work,
            executor,
            SupervisorLimits::default(),
        )
    }
    pub fn with_task_limits(
        binding: JournalCatalogBinding,
        journal: Arc<dyn CoordinatorJournal>,
        cleanup: Arc<dyn RunJournalCleanup>,
        encoder: Arc<dyn CoordinatorEventEncoder>,
        work: crate::CoordinatorPublicationBudgets,
        executor: Handle,
        limits: SupervisorLimits,
    ) -> Self {
        let tasks = TaskSupervisor::new(executor.clone(), limits);
        Self {
            binding,
            io: JournalIo::new(journal, cleanup, executor),
            encoder,
            work,
            runs: super::authority::RunAuthorities::default(),
            tasks,
            publication: None,
            summaries: None,
        }
    }
    pub fn with_publication(
        mut self,
        publication: Arc<super::publication::CoordinatorPublication>,
    ) -> Self {
        self.publication = Some(publication);
        self
    }
    /// Publishes chat summaries to the runner directory from the delivery sweep.
    pub fn with_summaries(
        mut self,
        summaries: Arc<super::summary_publication::CoordinatorSummaries>,
    ) -> Self {
        self.summaries = Some(summaries);
        self
    }
    /// Supply this exact tracked owner to retained native tools at composition.
    pub fn tool_tasks(&self) -> Arc<dyn nocterm_cloud_model::tool::ToolTaskSpawner> {
        Arc::new(self.tasks.clone())
    }
    pub fn task_snapshot(&self) -> TaskSnapshot {
        self.tasks.snapshot()
    }
    pub(super) fn tasks(&self) -> &TaskSupervisor {
        &self.tasks
    }
    pub fn background_journal(
        &self,
    ) -> Arc<dyn nocterm_cloud_model::tool::BackgroundCommandJournal> {
        Arc::new(super::facades::BackgroundJournal::new(self.clone()))
    }
    pub fn background_cleanup(
        &self,
    ) -> Arc<dyn nocterm_cloud_model::tool::BackgroundCommandCleanup> {
        Arc::new(super::facades::BackgroundCleanup::new(self.clone()))
    }
    pub fn background_authority(
        &self,
        source: Arc<dyn AuthenticatedCommandSource>,
        timer: Arc<dyn ToolTimer>,
    ) -> Result<Arc<dyn nocterm_cloud_model::tool::BackgroundCommandAuthority>, CoordinatorError>
    {
        Ok(Arc::new(super::authority::BackgroundAuthority::new(
            self.clone(),
            source,
            timer,
            self.runs.clone(),
        )?))
    }
    /// Actual native work includes a blocking job whose caller was cancelled.
    /// This observation is not itself a shutdown or safe-to-replay capability.
    pub fn in_flight_writes(&self) -> usize {
        self.io.in_flight_writes()
    }
    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub(super) fn io(&self) -> &JournalIo {
        &self.io
    }
    pub(super) fn publication_work(&self) -> &crate::CoordinatorPublicationBudgets {
        &self.work
    }
    pub(super) fn encoder(&self) -> &Arc<dyn CoordinatorEventEncoder> {
        &self.encoder
    }
}
/// Native capabilities supplied by their real owners; no service discovery or
/// default authentication/credential lookup exists in the coordinator.
pub struct CoordinatorExecution {
    providers: Arc<dyn CoordinatorProviders>,
    sessions: Arc<dyn SessionConsentConsumer>,
    tools: Arc<dyn RunToolRuntimeFactory>,
    arguments: Arc<dyn ArgumentFingerprint>,
    session_setup: Option<super::session_services::CoordinatorSessions>,
    retirement: Option<Arc<dyn ChatRetirement>>,
    approval: Option<Arc<dyn super::ToolApprovalPolicy>>,
}
impl CoordinatorExecution {
    pub fn new(
        providers: Arc<dyn CoordinatorProviders>,
        sessions: Arc<dyn SessionConsentConsumer>,
        tools: Arc<dyn RunToolRuntimeFactory>,
        arguments: Arc<dyn ArgumentFingerprint>,
    ) -> Self {
        Self {
            providers,
            sessions,
            tools,
            arguments,
            session_setup: None,
            retirement: None,
            approval: None,
        }
    }
    /// Lets the embedding runner decide which mutating calls need a person.
    /// Without it every mutating call asks.
    pub fn with_approval_policy(mut self, policy: Arc<dyn super::ToolApprovalPolicy>) -> Self {
        self.approval = Some(policy);
        self
    }
    /// Announces each deleted chat to the hub, which may reclaim its capacity.
    pub fn with_retirement(mut self, retirement: Arc<dyn ChatRetirement>) -> Self {
        self.retirement = Some(retirement);
        self
    }
    pub fn with_sessions(mut self, setup: super::session_services::CoordinatorSessions) -> Self {
        self.session_setup = Some(setup);
        self
    }
}
/// Explicit native composition. Constructor equality checks do not replace the
/// producer/cipher implementations' verification of their full configured pins.
pub struct CoordinatorServices {
    source: Arc<dyn AuthenticatedCommandSource>,
    storage: CoordinatorStorage,
    execution: CoordinatorExecution,
    identity: Arc<dyn CoordinatorIdentity>,
    timer: Arc<dyn ToolTimer>,
}
impl CoordinatorServices {
    pub fn new(
        source: Arc<dyn AuthenticatedCommandSource>,
        storage: CoordinatorStorage,
        execution: CoordinatorExecution,
        identity: Arc<dyn CoordinatorIdentity>,
        timer: Arc<dyn ToolTimer>,
    ) -> Result<Self, CoordinatorError> {
        if source.binding() != storage.binding() {
            return Err(CoordinatorError::StaleSource);
        }
        if execution
            .session_setup
            .as_ref()
            .is_some_and(|setup| setup.binding() != storage.binding())
        {
            return Err(CoordinatorError::StaleSource);
        }
        Ok(Self {
            source,
            storage,
            execution,
            identity,
            timer,
        })
    }
    pub(super) fn register(
        &self,
        run: &Arc<super::run::RunRecord>,
    ) -> Result<(), CoordinatorError> {
        self.storage.runs.register(run)
    }
    pub(super) fn stop_runs(&self) -> Vec<nocterm_cloud_model::authority::RunBinding> {
        self.storage.runs.stop_all()
    }
    pub(super) fn tasks(&self) -> &TaskSupervisor {
        self.storage.tasks()
    }
    pub(super) fn publication(&self) -> Option<&Arc<super::publication::CoordinatorPublication>> {
        self.storage.publication.as_ref()
    }
    pub(super) fn summaries(
        &self,
    ) -> Option<&Arc<super::summary_publication::CoordinatorSummaries>> {
        self.storage.summaries.as_ref()
    }
    pub(super) fn source(&self) -> &Arc<dyn AuthenticatedCommandSource> {
        &self.source
    }
    pub(super) fn io(&self) -> &JournalIo {
        self.storage.io()
    }
    pub(super) fn publication_work(&self) -> &crate::CoordinatorPublicationBudgets {
        self.storage.publication_work()
    }
    pub(super) fn encoder(&self) -> &Arc<dyn CoordinatorEventEncoder> {
        self.storage.encoder()
    }
    pub(super) fn providers(&self) -> &Arc<dyn CoordinatorProviders> {
        &self.execution.providers
    }
    pub(super) fn sessions(&self) -> &Arc<dyn SessionConsentConsumer> {
        &self.execution.sessions
    }
    pub(super) fn session_setup(
        &self,
    ) -> Result<&super::session_services::CoordinatorSessions, CoordinatorError> {
        self.execution
            .session_setup
            .as_ref()
            .ok_or(CoordinatorError::Unsupported)
    }
    pub(super) fn retirement(&self) -> Option<&Arc<dyn ChatRetirement>> {
        self.execution.retirement.as_ref()
    }
    pub(super) fn approval(&self) -> Option<&Arc<dyn super::ToolApprovalPolicy>> {
        self.execution.approval.as_ref()
    }
    pub(super) fn storage(&self) -> &CoordinatorStorage {
        &self.storage
    }
    pub(super) fn tools(&self) -> &Arc<dyn RunToolRuntimeFactory> {
        &self.execution.tools
    }
    pub(super) fn arguments(&self) -> &Arc<dyn ArgumentFingerprint> {
        &self.execution.arguments
    }
    pub(super) fn identity(&self) -> &Arc<dyn CoordinatorIdentity> {
        &self.identity
    }
    pub(super) fn timer(&self) -> &Arc<dyn ToolTimer> {
        &self.timer
    }
}
