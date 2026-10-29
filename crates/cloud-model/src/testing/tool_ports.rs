//! Shared in-memory collaborators do not prove native durability or authentication.
use crate::{
    authority::{ExecutionAuthority, SessionReceipt},
    identity::Timestamp,
    tool::{
        BackgroundCommand, BackgroundCommandJournal, BackgroundCommandOutcome, OpenedToolSession,
        ScopedSessionOpener, ScopedToolRequest, ToolAuthority, ToolExecutionError, ToolTaskSpawner,
        ToolTimer,
    },
};
use futures::{
    FutureExt as _,
    channel::oneshot,
    future::{BoxFuture, Shared},
};
use nocterm_session::Session;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

#[derive(Default)]
struct TimerState {
    now: u64,
    sleepers: Vec<(u64, oneshot::Sender<()>)>,
}
#[derive(Clone, Default)]
pub struct ManualToolTimer(Arc<Mutex<TimerState>>);
impl ManualToolTimer {
    pub fn set(&self, millis: u64) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.now = millis;
        let waiting = std::mem::take(&mut state.sleepers);
        for (deadline, sender) in waiting {
            if deadline <= millis {
                let _ = sender.send(());
            } else if !sender.is_canceled() {
                state.sleepers.push((deadline, sender));
            }
        }
    }
    pub fn advance(&self, millis: u64) {
        self.set(self.now().as_millis().saturating_add(millis));
    }
}
impl ToolTimer for ManualToolTimer {
    fn now(&self) -> Timestamp {
        Timestamp::from_millis(self.0.lock().unwrap_or_else(PoisonError::into_inner).now)
    }
    fn delay(&self, duration: Duration) -> BoxFuture<'static, ()> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let (sender, receiver) = oneshot::channel();
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        let deadline = state.now.saturating_add(millis);
        if deadline <= state.now {
            let _ = sender.send(());
        } else {
            state.sleepers.push((deadline, sender));
        }
        Box::pin(async move {
            let _ = receiver.await;
        })
    }
}

struct AuthorityState {
    expected: ExecutionAuthority,
    refusal: Option<ToolExecutionError>,
    checks: usize,
    background: Vec<ExecutionAuthority>,
}
pub struct ScriptedToolAuthority(Mutex<AuthorityState>);
impl ScriptedToolAuthority {
    pub fn new(expected: ExecutionAuthority) -> Self {
        Self(Mutex::new(AuthorityState {
            expected,
            refusal: None,
            checks: 0,
            background: Vec::new(),
        }))
    }
    pub fn refuse(&self, error: ToolExecutionError) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .refusal = Some(error);
    }
    /// Retained-job fixtures may keep several original grants simultaneously.
    /// This does not widen ordinary tool dispatch's one exact expected authority.
    pub fn allow_background(&self, expected: ExecutionAuthority) -> Result<(), ToolExecutionError> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.background.len() >= 32 {
            return Err(ToolExecutionError::Capacity);
        }
        state.background.push(expected);
        Ok(())
    }
    pub fn checks(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).checks
    }
}
impl ToolAuthority for ScriptedToolAuthority {
    fn check(&self, request: &ScopedToolRequest) -> Result<(), ToolExecutionError> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.checks += 1;
        if let Some(error) = &state.refusal {
            return Err(error.clone());
        }
        request
            .permit()
            .intent()
            .authority()
            .require_same(&state.expected)?;
        request.call().validate()?;
        Ok(())
    }
}

type BackgroundObservation = (BackgroundCommand, Option<BackgroundCommandOutcome>);
#[derive(Default)]
struct JournalState {
    commands: Vec<BackgroundObservation>,
    refusal: Option<ToolExecutionError>,
}
#[derive(Clone, Default)]
pub struct ScriptedBackgroundJournal(Arc<Mutex<JournalState>>);
impl ScriptedBackgroundJournal {
    pub fn observations(&self) -> Vec<BackgroundObservation> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .commands
            .clone()
    }
    pub fn refuse(&self, error: ToolExecutionError) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .refusal = Some(error);
    }
}
impl BackgroundCommandJournal for ScriptedBackgroundJournal {
    fn retain(
        &self,
        command: BackgroundCommand,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>> {
        let state = self.0.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(error) = &state.refusal {
                return Err(error.clone());
            }
            if let Some((existing, _)) = state
                .commands
                .iter()
                .find(|(old, _)| old.command() == command.command())
            {
                return if existing == &command {
                    Ok(())
                } else {
                    Err(ToolExecutionError::ArgumentMismatch)
                };
            }
            state.commands.push((command, None));
            Ok(())
        })
    }
    fn finish(
        &self,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>> {
        let state = self.0.clone();
        Box::pin(async move {
            let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(error) = &state.refusal {
                return Err(error.clone());
            }
            let (_, observed) = state
                .commands
                .iter_mut()
                .find(|(old, _)| old == &command)
                .ok_or(ToolExecutionError::ArgumentMismatch)?;
            if observed.is_some_and(|old| old != outcome) {
                return Err(ToolExecutionError::ArgumentMismatch);
            }
            *observed = Some(outcome);
            Ok(())
        })
    }
}

type ToolTask = BoxFuture<'static, Result<(), ToolExecutionError>>;
#[derive(Default)]
pub struct QueuedToolTasks(Mutex<TaskState>);
#[derive(Default)]
struct TaskState {
    tasks: Vec<ToolTask>,
    refusal: Option<ToolExecutionError>,
}
impl QueuedToolTasks {
    pub fn refuse(&self, error: ToolExecutionError) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .refusal = Some(error);
    }
    pub fn take(&self) -> Vec<ToolTask> {
        std::mem::take(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner).tasks)
    }
}
impl ToolTaskSpawner for QueuedToolTasks {
    fn spawn(&self, task: ToolTask) -> Result<(), ToolExecutionError> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(error) = &state.refusal {
            return Err(error.clone());
        }
        state.tasks.push(task);
        Ok(())
    }
}

#[derive(Default)]
struct OpenerState {
    sessions: VecDeque<(SessionReceipt, Session)>,
    requests: Vec<ScopedToolRequest>,
    gate: Option<Shared<BoxFuture<'static, ()>>>,
}
#[derive(Clone, Default)]
pub struct ScriptedSessionOpener(Arc<Mutex<OpenerState>>);
impl ScriptedSessionOpener {
    pub fn suspend(&self, wait: impl std::future::Future<Output = ()> + Send + 'static) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).gate = Some(wait.boxed().shared());
    }
    pub fn queue(&self, receipt: SessionReceipt, session: Session) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sessions
            .push_back((receipt, session));
    }
    pub fn requests(&self) -> Vec<ScopedToolRequest> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .requests
            .clone()
    }
}
impl ScopedSessionOpener for ScriptedSessionOpener {
    fn open(
        &self,
        request: ScopedToolRequest,
    ) -> BoxFuture<'static, Result<OpenedToolSession, ToolExecutionError>> {
        let state = self.0.clone();
        Box::pin(async move {
            let (receipt, session, gate) = {
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                state.requests.push(request);
                let (receipt, session) = state
                    .sessions
                    .pop_front()
                    .ok_or(ToolExecutionError::NeedsUnlock)?;
                (receipt, session, state.gate.clone())
            };
            if let Some(gate) = gate {
                gate.await;
            }
            Ok(OpenedToolSession::new(receipt, session))
        })
    }
}

impl crate::tool::BackgroundCommandAuthority for ScriptedToolAuthority {
    fn check(&self, command: &BackgroundCommand) -> Result<(), ToolExecutionError> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.checks += 1;
        if let Some(error) = &state.refusal {
            return Err(error.clone());
        }
        let actual = command.dispatch().intent().authority();
        if state.background.iter().any(|expected| actual == expected) {
            return Ok(());
        }
        actual.require_same(&state.expected)?;
        Ok(())
    }
}
impl crate::tool::BackgroundCommandCleanup for ScriptedBackgroundJournal {
    fn finish(
        &self,
        command: BackgroundCommand,
        outcome: BackgroundCommandOutcome,
    ) -> BoxFuture<'static, Result<(), ToolExecutionError>> {
        BackgroundCommandJournal::finish(self, command, outcome)
    }
}

/// Host-supplied opaque handles, consumed once by each factory attachment.
pub struct ScriptedSessionHandles(Mutex<VecDeque<crate::identity::RemoteSessionId>>);
impl ScriptedSessionHandles {
    pub fn new(handles: impl IntoIterator<Item = crate::identity::RemoteSessionId>) -> Self {
        Self(Mutex::new(handles.into_iter().collect()))
    }
}
impl crate::tool::ToolSessionHandleSource for ScriptedSessionHandles {
    fn next(
        &self,
        _run: &crate::authority::RunBinding,
    ) -> Result<crate::identity::RemoteSessionId, ToolExecutionError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or(ToolExecutionError::Capacity)
    }
}
