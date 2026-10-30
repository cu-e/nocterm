//! The actual coordinator-owned task, background and identity collaborators.
use nocterm_cloud_model::{
    authority::RunBinding,
    coordinator::{AuthenticatedCommandSource, CoordinatorError, CoordinatorIdentity},
    identity::RemoteSessionId,
    journal::PublicationWorkBudget,
    tool::{
        BackgroundCommandAuthority, BackgroundCommandCleanup, BackgroundCommandJournal,
        ToolExecutionError, ToolSessionHandleSource, ToolTaskSpawner, ToolTimer,
    },
};
use nocterm_cloud_runner::coordinator::CoordinatorStorage;
use nocterm_core::SharedCause;
use nocterm_hub_io::cloud::{RunnerCommandAttachments, RunnerHubConnection};
use std::sync::Arc;

pub(crate) struct SessionBrokerSetup {
    connection: Arc<RunnerHubConnection>,
    attachments: Arc<RunnerCommandAttachments>,
    source: Arc<dyn AuthenticatedCommandSource>,
    timer: Arc<dyn ToolTimer>,
    identity: Arc<dyn CoordinatorIdentity>,
    work: Arc<dyn PublicationWorkBudget>,
    tasks: Arc<dyn ToolTaskSpawner>,
    journal: Arc<dyn BackgroundCommandJournal>,
    authority: Arc<dyn BackgroundCommandAuthority>,
    cleanup: Arc<dyn BackgroundCommandCleanup>,
}
impl SessionBrokerSetup {
    pub(crate) fn new(
        connection: Arc<RunnerHubConnection>,
        attachments: Arc<RunnerCommandAttachments>,
        storage: &CoordinatorStorage,
        source: Arc<dyn AuthenticatedCommandSource>,
        timer: Arc<dyn ToolTimer>,
        identity: Arc<dyn CoordinatorIdentity>,
        work: Arc<dyn PublicationWorkBudget>,
    ) -> Result<Self, CoordinatorError> {
        if connection.binding() != storage.binding() || source.binding() != storage.binding() {
            return Err(CoordinatorError::StaleSource);
        }
        connection.check_runtime_source().map_err(backend)?;
        source.check_current()?;
        let authority = storage.background_authority(source.clone(), timer.clone())?;
        Ok(Self {
            connection,
            attachments,
            source,
            timer,
            identity,
            work,
            tasks: storage.tool_tasks(),
            journal: storage.background_journal(),
            authority,
            cleanup: storage.background_cleanup(),
        })
    }
    pub(super) fn connection(&self) -> &Arc<RunnerHubConnection> {
        &self.connection
    }
    pub(super) fn attachments(&self) -> &Arc<RunnerCommandAttachments> {
        &self.attachments
    }
    pub(super) fn source(&self) -> &Arc<dyn AuthenticatedCommandSource> {
        &self.source
    }
    pub(super) fn timer(&self) -> &Arc<dyn ToolTimer> {
        &self.timer
    }
    pub(super) fn identity(&self) -> &Arc<dyn CoordinatorIdentity> {
        &self.identity
    }
    pub(super) fn work(&self) -> &Arc<dyn PublicationWorkBudget> {
        &self.work
    }
    pub(super) fn tasks(&self) -> &Arc<dyn ToolTaskSpawner> {
        &self.tasks
    }
    pub(super) fn journal(&self) -> &Arc<dyn BackgroundCommandJournal> {
        &self.journal
    }
    pub(super) fn authority(&self) -> &Arc<dyn BackgroundCommandAuthority> {
        &self.authority
    }
    pub(super) fn cleanup(&self) -> &Arc<dyn BackgroundCommandCleanup> {
        &self.cleanup
    }
    pub(super) fn handles(self: &Arc<Self>) -> Arc<dyn ToolSessionHandleSource> {
        Arc::new(SessionHandles(self.clone()))
    }
}
struct SessionHandles(Arc<SessionBrokerSetup>);
impl ToolSessionHandleSource for SessionHandles {
    fn next(&self, run: &RunBinding) -> Result<RemoteSessionId, ToolExecutionError> {
        let next = || -> Result<_, CoordinatorError> {
            self.0.source.check_current()?;
            if !self.0.source.binding().contains(&run.stream()) {
                return Err(CoordinatorError::StaleSource);
            }
            let id = self.0.identity.fresh()?;
            self.0.source.check_current()?;
            Ok(RemoteSessionId::from_bytes(id))
        };
        next().map_err(|error| ToolExecutionError::Execution(SharedCause::new(error)))
    }
}
fn backend(error: impl std::error::Error + Send + Sync + 'static) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}
