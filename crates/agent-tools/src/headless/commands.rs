//! Shared Jobs retain native execution; background lifecycle is durably acknowledged.
use super::{HeadlessError, physical::PhysicalTerminal, presentation};
use futures::{FutureExt, future::BoxFuture};
use nocterm_cloud_model::tool::{
    BackgroundCommand, BackgroundCommandAuthority, BackgroundCommandCleanup,
    BackgroundCommandJournal, BackgroundCommandOutcome, ScopedToolRequest, ToolAuthority,
    ToolEffectStatus, ToolExecutionError, ToolTaskSpawner, ToolTimer,
};
use nocterm_core::SharedCause;
use nocterm_session::{
    ExecRequest, HostExec,
    command_jobs::{CommandJobs, CommandSnapshot, CommandState},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

struct CommandObservation {
    original: BackgroundCommand,
    native: String,
    outcome: Arc<Mutex<Option<BackgroundCommandOutcome>>>,
    owner: RuntimeOwner,
}
/// Only the native runtime creates this local cleanup identity. It is neither
/// serialized nor inferred from a run, grant, session or intent coordinate.
#[derive(Clone)]
pub(super) struct RuntimeOwner(Arc<()>);
impl RuntimeOwner {
    pub(super) fn new() -> Self {
        Self(Arc::new(()))
    }
    fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
pub(super) struct BackgroundJobs {
    jobs: Box<dyn CommandJobs>,
    commands: BTreeMap<String, CommandObservation>,
}
impl BackgroundJobs {
    pub(super) fn new() -> Self {
        Self {
            jobs: Box::<crate::jobs::Jobs>::default(),
            commands: BTreeMap::new(),
        }
    }
    pub(super) fn start(
        &mut self,
        request: &ScopedToolRequest,
        executor: Arc<dyn HostExec>,
        owner: RuntimeOwner,
        deadline: BoxFuture<'static, ()>,
    ) -> Result<PreparedCommand, ToolExecutionError> {
        let nocterm_ai::TerminalCall::ExecCommand(call) = request.call() else {
            return Err(ToolExecutionError::ArgumentMismatch);
        };
        let id = opaque_command(request);
        // A repeated intent must never replace the original observation, even
        // before its durable retention rejects a changed runtime or command.
        if self.commands.contains_key(&id) {
            return Err(ToolExecutionError::ArgumentMismatch);
        }
        self.commands.retain(|_, observed| {
            observed
                .outcome
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_none()
                || self
                    .jobs
                    .snapshot(&observed.native, observed.original.terminal())
                    .is_ok()
        });
        if self.commands.len() >= 20 {
            return Err(ToolExecutionError::Capacity);
        }
        let start = self
            .jobs
            .start(
                call.terminal_id.clone(),
                executor,
                ExecRequest {
                    program: call.program.clone(),
                    args: call.args.clone(),
                    stdin: call.stdin.as_ref().map(|text| text.as_bytes().to_vec()),
                },
                deadline,
            )
            .map_err(job_error)?;
        let (native, run) = start.into_run();
        let original = BackgroundCommand::new(
            request.permit().clone(),
            call.terminal_id.clone(),
            id.clone(),
        )?;
        let outcome = Arc::new(Mutex::new(None));
        self.commands.insert(
            id.clone(),
            CommandObservation {
                original: original.clone(),
                native,
                outcome: outcome.clone(),
                owner,
            },
        );
        Ok(PreparedCommand {
            id,
            original,
            outcome,
            run,
        })
    }
    pub(super) fn snapshot(
        &self,
        id: &str,
        receipt: &nocterm_cloud_model::authority::SessionReceipt,
        cancel: bool,
    ) -> Result<(serde_json::Value, ToolEffectStatus), ToolExecutionError> {
        let entry = self
            .commands
            .get(id)
            .filter(|entry| entry.original.dispatch().intent().authority().session() == receipt)
            .ok_or(HeadlessError::StaleSession)?;
        let snapshot = if cancel {
            self.jobs.cancel(&entry.native, entry.original.terminal())
        } else {
            self.jobs.snapshot(&entry.native, entry.original.terminal())
        }
        .map_err(job_error)?;
        let outcome = *entry.outcome.lock().unwrap_or_else(PoisonError::into_inner);
        let effect = match outcome {
            Some(BackgroundCommandOutcome::ObservedExit(_)) => ToolEffectStatus::Observed,
            Some(BackgroundCommandOutcome::Unknown) => ToolEffectStatus::Unknown,
            None => ToolEffectStatus::Background(Arc::new(entry.original.clone())),
        };
        Ok((presentation::command(id, &snapshot), effect))
    }
    pub(super) fn observation(&self, id: &str) -> Result<CommandSnapshot, ToolExecutionError> {
        let entry = self.commands.get(id).ok_or(HeadlessError::StaleSession)?;
        self.jobs
            .snapshot(&entry.native, entry.original.terminal())
            .map_err(job_error)
    }
    pub(super) fn cancel(&self, id: &str) {
        if let Some(entry) = self.commands.get(id) {
            let _ = self.jobs.cancel(&entry.native, entry.original.terminal());
        }
    }
    pub(super) fn cancel_owner(&self, owner: &RuntimeOwner) {
        for entry in self
            .commands
            .values()
            .filter(|entry| entry.owner.same_as(owner))
        {
            let _ = self.jobs.cancel(&entry.native, entry.original.terminal());
        }
    }
    pub(super) fn cancel_all(&self) {
        self.jobs.cancel_all();
    }
}

pub(super) struct PreparedCommand {
    id: String,
    original: BackgroundCommand,
    outcome: Arc<Mutex<Option<BackgroundCommandOutcome>>>,
    run: BoxFuture<'static, ()>,
}
impl PreparedCommand {
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) async fn launch(
        self,
        supervisor: CommandSupervisor,
        spawner: &dyn ToolTaskSpawner,
        request: ScopedToolRequest,
    ) -> Result<(), ToolExecutionError> {
        if let Err(error) = supervisor.journal.retain(self.original.clone()).await {
            supervisor
                .jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .cancel(&self.id);
            return Err(error);
        }
        let id = self.id.clone();
        let cancel_jobs = supervisor.jobs.clone();
        let work = async move {
            let mut run = self.run;
            let mut admitted = false;
            let execution = futures::future::poll_fn(|cx| {
                // The physical lock serializes retirement with this native poll.
                // A foreground view is required only for its first dispatch poll.
                let entry = supervisor
                    .physical
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if let Err(error) = entry.check(request.permit().intent().authority()) {
                    return std::task::Poll::Ready(Err(error.into()));
                }
                let check = if admitted {
                    supervisor.observations.as_ref().map_or_else(
                        || supervisor.authority.check(&request),
                        |authority| authority.check(&self.original),
                    )
                } else {
                    supervisor.authority.check(&request)
                };
                if let Err(error) = check {
                    return std::task::Poll::Ready(Err(error));
                }
                admitted = true;
                run.as_mut().poll(cx).map(Ok)
            })
            .await;
            drop(run);
            let outcome = if execution.is_err() {
                supervisor
                    .jobs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .cancel(&self.id);
                BackgroundCommandOutcome::Unknown
            } else {
                let snapshot = supervisor
                    .jobs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .observation(&self.id)?;
                if snapshot.state() == CommandState::Exited {
                    snapshot.exit().and_then(|exit| exit.status).map_or(
                        BackgroundCommandOutcome::Unknown,
                        BackgroundCommandOutcome::ObservedExit,
                    )
                } else {
                    BackgroundCommandOutcome::Unknown
                }
            };
            // Persist the actual historical fact through the explicitly injected
            // cleanup writer, without renewing old dispatch authority or I/O.
            supervisor.finish(self.original, outcome).await?;
            *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
            execution
        }
        .boxed();
        if let Err(error) = spawner.spawn(work) {
            cancel_jobs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .cancel(&id);
            return Err(error);
        }
        Ok(())
    }
}

pub(super) struct CommandSupervisor {
    jobs: Arc<Mutex<BackgroundJobs>>,
    journal: Arc<dyn BackgroundCommandJournal>,
    authority: Arc<dyn ToolAuthority>,
    physical: Arc<Mutex<PhysicalTerminal>>,
    observations: Option<Arc<dyn BackgroundCommandAuthority>>,
    cleanup: Option<Arc<dyn BackgroundCommandCleanup>>,
}
impl CommandSupervisor {
    pub(super) fn new(
        jobs: Arc<Mutex<BackgroundJobs>>,
        journal: Arc<dyn BackgroundCommandJournal>,
        authority: Arc<dyn ToolAuthority>,
        physical: Arc<Mutex<PhysicalTerminal>>,
        retained: Option<&super::retained::RetainedOwner>,
    ) -> Self {
        Self {
            jobs,
            journal,
            authority,
            physical,
            observations: retained.map(super::retained::RetainedOwner::observations),
            cleanup: retained.map(super::retained::RetainedOwner::cleanup),
        }
    }
    async fn finish(
        &self,
        original: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> Result<(), ToolExecutionError> {
        match &self.cleanup {
            Some(cleanup) => cleanup.finish(original, outcome).await,
            None => self.journal.finish(original, outcome).await,
        }
    }
}

pub(super) fn deadline(
    request: ScopedToolRequest,
    authority: Arc<dyn ToolAuthority>,
    timer: Arc<dyn ToolTimer>,
    observations: Option<Arc<dyn BackgroundCommandAuthority>>,
) -> BoxFuture<'static, ()> {
    let timeout = match request.call() {
        nocterm_ai::TerminalCall::ExecCommand(call) => call.timeout_ms.unwrap_or(30_000),
        _ => 30_000,
    };
    async move {
        let hard = timer.delay(Duration::from_millis(timeout)).fuse();
        let revoked = async {
            loop {
                timer.delay(Duration::from_millis(50)).await;
                let current = match &observations {
                    Some(observer) => BackgroundCommand::new(
                        request.permit().clone(),
                        request.call().terminal_id().unwrap_or_default().to_owned(),
                        opaque_command(&request),
                    )
                    .and_then(|command| observer.check(&command)),
                    None => authority.check(&request),
                };
                if current.is_err() {
                    break;
                }
            }
        }
        .fuse();
        futures::pin_mut!(hard, revoked);
        futures::select_biased! { () = hard => (), () = revoked => () }
    }
    .boxed()
}
fn opaque_command(request: &ScopedToolRequest) -> String {
    let mut value = String::from("c-");
    for byte in request.permit().intent().id().as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(value, "{byte:02x}");
    }
    value
}
fn job_error(error: nocterm_session::command_jobs::JobError) -> ToolExecutionError {
    match error {
        nocterm_session::command_jobs::JobError::Busy => ToolExecutionError::Capacity,
        error => ToolExecutionError::Execution(SharedCause::new(error)),
    }
}
