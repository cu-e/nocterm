//! Real mixed runner inbox admission; transport storage is never execution acceptance.
mod acknowledge;
mod attachments;
mod commands;
mod cursor;
mod material_store;
mod receive;
use super::RunnerHubConnection;
pub use attachments::{
    BootAnnouncement, RunnerCommandAttachments, RunnerProviderBootOffer,
    RunnerProviderCredentialSource, RunnerSshAnswer, RunnerStagedProvider,
};
use cursor::InboxCursor;
use futures::future::BoxFuture;
pub use material_store::{MaterialStoreError, RunnerMaterialStore};
use nocterm_cloud_model::{
    coordinator::{
        AuthenticatedCommandSource, CommandAcknowledgement, CommandInput, CommandReceiveBudget,
        CommandReply, CoordinatorError, ResidentCommand, RunDestination,
    },
    identity::{CloudChatId, CommandId},
    journal::{JournalCatalogBinding, JournalControlReader},
    tool::ToolTimer,
};
use nocterm_core::{SharedCause, allocation::AllocationLease};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

/// Required account pin and explicit default runner principal. The default is
/// used only when the authenticated Submit payload deliberately omits a target.
pub struct RunnerInboxPrincipal {
    account: [u8; 32],
    default: RunDestination,
}
impl RunnerInboxPrincipal {
    pub fn new(account: [u8; 32], default: RunDestination) -> Self {
        Self { account, default }
    }
}
#[derive(Clone, Copy)]
pub enum InboxResume {
    CreateNew,
    OpenExisting,
}
pub struct RunnerInboxOptions {
    directory: PathBuf,
    resume: InboxResume,
    interval: Duration,
    timeout: Duration,
    probe_every: usize,
    maximum: usize,
}
impl RunnerInboxOptions {
    pub fn new(directory: PathBuf, resume: InboxResume) -> Self {
        Self {
            directory,
            resume,
            interval: Duration::from_millis(100),
            timeout: Duration::from_secs(5),
            probe_every: 16,
            maximum: 256,
        }
    }
    pub fn with_limits(
        mut self,
        interval: Duration,
        timeout: Duration,
        probe_every: usize,
        maximum: usize,
    ) -> Result<Self, CoordinatorError> {
        if interval.is_zero()
            || interval > Duration::from_secs(30)
            || timeout.is_zero()
            || timeout > Duration::from_secs(60)
            || probe_every == 0
            || probe_every > 128
            || maximum == 0
            || maximum > 256
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        self.interval = interval;
        self.timeout = timeout;
        self.probe_every = probe_every;
        self.maximum = maximum;
        Ok(self)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnerInboxFault {
    DeliveryUnconfirmed,
    NeedsRunnerAccess,
    StorageUnavailable,
    InvalidCommand,
    HubGapOrRollback,
    Authentication,
}
impl RunnerInboxFault {
    fn fatal(self) -> bool {
        matches!(self, Self::HubGapOrRollback | Self::Authentication)
    }
}
struct PendingDelivery {
    command: CommandAcknowledgement,
    cursor: u64,
    resident: Weak<
        nocterm_core::allocation::Allocated<nocterm_cloud_model::coordinator::ReceivedCommand>,
    >,
    _allocation: AllocationLease,
}
struct InboxProgress {
    cursor: InboxCursor,
    forward: u64,
    probes: usize,
    pending: BTreeMap<(CloudChatId, CommandId), PendingDelivery>,
    budget: Option<CommandReceiveBudget>,
    hub_confirmed: u64,
    fault: Option<RunnerInboxFault>,
}
struct InboxOwner {
    connection: Arc<RunnerHubConnection>,
    material: Arc<RunnerMaterialStore>,
    principal: RunnerInboxPrincipal,
    journal: Arc<dyn JournalControlReader>,
    attachments: Arc<RunnerCommandAttachments>,
    timer: Arc<dyn ToolTimer>,
    options: RunnerInboxOptions,
    progress: Mutex<InboxProgress>,
    receive: tokio::sync::Mutex<()>,
    acknowledge: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct RunnerInbox {
    owner: Arc<InboxOwner>,
}
impl RunnerInbox {
    /// Explicit fresh-identity initialization. Daemon restore always uses
    /// OpenExisting; a missing cursor never selects this creation path.
    pub fn initialize_storage(
        directory: &std::path::Path,
        identity: &super::CloudRunnerIdentity,
    ) -> Result<(), CoordinatorError> {
        let public = identity.public_identity().map_err(backend)?;
        let cursor =
            InboxCursor::open(directory, identity.binding(), public.encryption_key(), true)
                .map_err(backend)?;
        drop(cursor);
        Ok(())
    }
    pub fn new(
        connection: Arc<RunnerHubConnection>,
        material: Arc<RunnerMaterialStore>,
        principal: RunnerInboxPrincipal,
        journal: Arc<dyn JournalControlReader>,
        attachments: Arc<RunnerCommandAttachments>,
        timer: Arc<dyn ToolTimer>,
        options: RunnerInboxOptions,
    ) -> Result<Self, CoordinatorError> {
        if connection.binding() != material.binding()
            || !connection.shares_identity(material.identity())
            || !attachments.shares_owners(&connection, &material, &timer)
            || <[u8; 32]>::from(Sha256::digest(principal.account))
                != *material.binding().scope().account().key().as_bytes()
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        let identity = material.identity().public_identity().map_err(backend)?;
        let cursor = InboxCursor::open(
            &options.directory,
            material.binding(),
            identity.encryption_key(),
            matches!(options.resume, InboxResume::CreateNew),
        )
        .map_err(backend)?;
        let forward = cursor.confirmed();
        Ok(Self {
            owner: Arc::new(InboxOwner {
                connection,
                material,
                principal,
                journal,
                attachments,
                timer,
                options,
                progress: Mutex::new(InboxProgress {
                    cursor,
                    forward,
                    probes: 0,
                    pending: BTreeMap::new(),
                    budget: None,
                    hub_confirmed: 0,
                    fault: None,
                }),
                receive: tokio::sync::Mutex::new(()),
                acknowledge: tokio::sync::Mutex::new(()),
            }),
        })
    }
    pub fn fault_state(&self) -> Result<Option<RunnerInboxFault>, CoordinatorError> {
        Ok(self
            .owner
            .progress
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?
            .fault)
    }
    pub fn attachments(&self) -> Arc<RunnerCommandAttachments> {
        self.owner.attachments.clone()
    }
}
impl InboxOwner {
    fn check_current(&self) -> Result<(), CoordinatorError> {
        self.connection.check_runtime_source().map_err(backend)?;
        self.attachments.check_current()?;
        if self
            .progress
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?
            .fault
            .is_some_and(RunnerInboxFault::fatal)
        {
            return Err(CoordinatorError::StaleSource);
        }
        Ok(())
    }
    fn fault(&self, fault: RunnerInboxFault) {
        if let Ok(mut state) = self.progress.lock() {
            if !state.fault.is_some_and(RunnerInboxFault::fatal) {
                state.fault = Some(fault);
            }
        }
    }
    async fn io<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, CoordinatorError>>,
    ) -> Result<T, CoordinatorError> {
        self.check_current()?;
        let response = tokio::time::timeout(self.options.timeout, future).await;
        self.check_current()?;
        response.map_err(|_| CoordinatorError::Backend(SharedCause::new(InboxTimeout)))?
    }
    fn remember(&self, command: &ResidentCommand, cursor: u64) -> Result<(), CoordinatorError> {
        self.prune_pending()?;
        let mut state = self
            .progress
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        let key = (
            command.value().stream().chat(),
            command.value().acceptance().id(),
        );
        if state.pending.contains_key(&key) {
            return Err(CoordinatorError::Overloaded);
        }
        if state.pending.len() >= self.options.maximum {
            return Err(CoordinatorError::Overloaded);
        }
        state.pending.insert(
            key,
            PendingDelivery {
                command: command.value().acknowledgement(),
                cursor,
                resident: Arc::downgrade(command),
                _allocation: command.allocation().clone(),
            },
        );
        Ok(())
    }
    fn prune_pending(&self) -> Result<(), CoordinatorError> {
        for _ in 0..self.options.maximum {
            let removed = {
                let mut state = self
                    .progress
                    .lock()
                    .map_err(|_| CoordinatorError::NativeTaskFailed)?;
                let key = state
                    .pending
                    .iter()
                    .find(|(_, entry)| entry.resident.strong_count() == 0)
                    .map(|(key, _)| *key);
                key.and_then(|key| state.pending.remove(&key))
            };
            let Some(entry) = removed else {
                break;
            };
            drop(entry);
        }
        Ok(())
    }
    fn inflight(&self, command: &CommandAcknowledgement) -> Result<bool, CoordinatorError> {
        self.prune_pending()?;
        let state = self
            .progress
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        match state
            .pending
            .get(&(command.stream().chat(), command.acceptance().id()))
        {
            Some(entry)
                if entry.command.stream() == command.stream()
                    && entry.command.acceptance() == command.acceptance() =>
            {
                Ok(true)
            }
            Some(_) => Err(CoordinatorError::Conflict),
            None => Ok(false),
        }
    }
}
impl AuthenticatedCommandSource for RunnerInbox {
    fn binding(&self) -> &JournalCatalogBinding {
        self.owner.material.binding()
    }
    fn check_current(&self) -> Result<(), CoordinatorError> {
        self.owner.check_current()
    }
    fn receive(
        &self,
        budget: CommandReceiveBudget,
    ) -> BoxFuture<'static, Result<Option<CommandInput>, CoordinatorError>> {
        let owner = self.owner.clone();
        Box::pin(async move { receive::receive(owner, budget).await })
    }
    fn acknowledge(
        &self,
        command: CommandAcknowledgement,
        reply: CommandReply,
        allocation: AllocationLease,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        let owner = self.owner.clone();
        Box::pin(async move { acknowledge::acknowledge(owner, command, reply, allocation).await })
    }
}
#[derive(Debug, thiserror::Error)]
#[error("the bounded original runner inbox request timed out")]
struct InboxTimeout;
fn backend(error: impl std::error::Error + Send + Sync + 'static) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}
