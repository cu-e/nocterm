//! One original run's initial connection and post-durability activation cutoff.
use super::{control::RunControl, log::ChatLog};
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, RunBinding, TargetBinding},
    coordinator::{
        AuthenticatedCommandSource, CoordinatorError, PublishedRunSession, SessionAttempt,
        SessionObservation, SessionOutcome, SessionRequest, SessionStartup,
    },
    identity::{AttemptId, Timestamp},
    tool::ToolTimer,
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use parking_lot::Mutex;
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::watch;

struct ActiveAttempt {
    request: Arc<Allocated<SessionRequest>>,
    generation: Arc<()>,
    cancellation: Option<Box<dyn nocterm_cloud_model::coordinator::SessionAttemptCancellation>>,
}
struct LiveSession {
    generation: Arc<()>,
    fact: Arc<Allocated<SessionObservation>>,
}
enum SessionPhase {
    Empty,
    Pending(ActiveAttempt),
    Durable(ActiveAttempt, Arc<Allocated<SessionObservation>>),
    Live(LiveSession),
    Lost,
    Retired,
}
impl SessionPhase {
    fn active(&self) -> Option<(&Arc<()>, &SessionAttempt)> {
        match self {
            Self::Pending(active) | Self::Durable(active, _) => {
                Some((&active.generation, active.request.value().binding()))
            }
            Self::Live(live) => Some((&live.generation, live.fact.value().binding())),
            _ => None,
        }
    }
}
struct SessionState {
    phase: SessionPhase,
    seen: BTreeSet<AttemptId>,
    observed: Option<Timestamp>,
}
pub(super) struct RunSessionOwner {
    run: RunBinding,
    target: TargetBinding,
    authentication: AuthenticationBinding,
    state: Mutex<SessionState>,
    changed: watch::Sender<()>,
    wake: Arc<AtomicBool>,
}
/// Native pointer identity stays private. Repeated descriptive attempt/run IDs
/// can never complete a replacement attempt.
#[derive(Clone)]
pub(super) struct SessionTicket {
    owner: Weak<RunSessionOwner>,
    binding: Arc<Allocated<SessionAttempt>>,
    generation: Arc<()>,
    startup: SessionStartup,
}
impl SessionTicket {
    pub(super) fn binding(&self) -> &SessionAttempt {
        self.binding.value()
    }
    pub(super) fn startup(&self) -> SessionStartup {
        self.startup
    }
    pub(super) fn check_pending(&self, timer: &dyn ToolTimer) -> Result<(), CoordinatorError> {
        let owner = self.owner.upgrade().ok_or(CoordinatorError::Stopped)?;
        let mut state = owner.state.lock();
        let now = timer.now();
        owner.require_current(&state, self)?;
        if !matches!(state.phase, SessionPhase::Pending(_)) {
            return Err(CoordinatorError::Conflict);
        }
        owner.check_time(&mut state, now)?;
        self.binding().require_time(now)
    }
    pub(super) fn check_connection(&self, timer: &dyn ToolTimer) -> Result<(), CoordinatorError> {
        let owner = self.owner.upgrade().ok_or(CoordinatorError::Stopped)?;
        let mut state = owner.state.lock();
        let now = timer.now();
        owner.require_current(&state, self)?;
        owner.check_time(&mut state, now)?;
        // The initial device consent expires; an established connection/rekey
        // instead retains the original actual runtime and physical generation.
        if !matches!(state.phase, SessionPhase::Live(_)) {
            self.binding().require_time(now)?;
        }
        Ok(())
    }
    pub(super) fn require_authentication(
        &self,
        request: &nocterm_cloud_model::login::LoginProofRequest,
    ) -> Result<(), CoordinatorError> {
        let owner = self.owner.upgrade().ok_or(CoordinatorError::Stopped)?;
        let state = owner.state.lock();
        owner.require_current(&state, self)?;
        let SessionPhase::Pending(active) = &state.phase else {
            return Err(CoordinatorError::Conflict);
        };
        if active.request.value().ssh_consent() != Some(request.binding()) {
            return Err(CoordinatorError::Conflict);
        }
        Ok(())
    }
    fn needs_run_control(&self) -> Result<bool, CoordinatorError> {
        let owner = self.owner.upgrade().ok_or(CoordinatorError::Stopped)?;
        let state = owner.state.lock();
        owner.require_current(&state, self)?;
        Ok(!matches!(state.phase, SessionPhase::Live(_)))
    }
    pub(super) async fn ended(&self) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let mut changed = owner.changed.subscribe();
        loop {
            if owner.require_current(&owner.state.lock(), self).is_err() {
                return;
            }
            if changed.changed().await.is_err() {
                return;
            }
        }
    }
}
impl RunSessionOwner {
    /// `wake` is the same original RunRecord flag observed by the actor queue.
    pub(super) fn new(
        run: RunBinding,
        target: TargetBinding,
        authentication: AuthenticationBinding,
        wake: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let (changed, _) = watch::channel(());
        Arc::new(Self {
            run,
            target,
            authentication,
            state: Mutex::new(SessionState {
                phase: SessionPhase::Empty,
                seen: BTreeSet::new(),
                observed: None,
            }),
            changed,
            wake,
        })
    }
    pub(super) fn begin(
        self: &Arc<Self>,
        request: Arc<Allocated<SessionRequest>>,
        resident: AllocationLease,
        timer: &dyn ToolTimer,
    ) -> Result<SessionTicket, CoordinatorError> {
        if resident.bytes() < session_descriptor_bytes() || resident.payloads() == 0 {
            return Err(CoordinatorError::Overloaded);
        }
        let binding = request.value().binding();
        if binding.run() != &self.run
            || binding.target() != &self.target
            || binding.authentication() != &self.authentication
        {
            return Err(CoordinatorError::Conflict);
        }
        let mut state = self.state.lock();
        let now = timer.now();
        binding.require_time(now)?;
        self.check_time(&mut state, now)?;
        if matches!(state.phase, SessionPhase::Retired | SessionPhase::Live(_))
            || state.seen.contains(&binding.attempt())
        {
            return Err(CoordinatorError::Conflict);
        }
        if state.seen.len() >= nocterm_cloud_model::login::MAX_LOGIN_CONSENTS {
            return Err(CoordinatorError::Overloaded);
        }
        state.seen.insert(binding.attempt());
        let binding = Arc::new(Allocated::new(binding.clone(), resident));
        let generation = Arc::new(());
        let previous = std::mem::replace(
            &mut state.phase,
            SessionPhase::Pending(ActiveAttempt {
                request: request.clone(),
                generation: generation.clone(),
                cancellation: None,
            }),
        );
        self.changed.send_replace(());
        drop(state);
        if let SessionPhase::Pending(active) | SessionPhase::Durable(active, _) = previous
            && let Some(cancellation) = active.cancellation
        {
            cancellation.cancel();
        }
        Ok(SessionTicket {
            owner: Arc::downgrade(self),
            binding,
            generation,
            startup: request.value().startup(),
        })
    }
    pub(super) fn attach_cancellation(
        &self,
        ticket: &SessionTicket,
        cancellation: Box<dyn nocterm_cloud_model::coordinator::SessionAttemptCancellation>,
    ) {
        let mut state = self.state.lock();
        if self.require_current(&state, ticket).is_ok() {
            match &mut state.phase {
                SessionPhase::Pending(active) | SessionPhase::Durable(active, _) => {
                    active.cancellation = Some(cancellation);
                    return;
                }
                _ => {}
            }
        }
        drop(state);
        cancellation.cancel();
    }
    pub(super) fn check_begin(
        &self,
        request: &SessionRequest,
        timer: &dyn ToolTimer,
    ) -> Result<(), CoordinatorError> {
        let binding = request.binding();
        let mut state = self.state.lock();
        let now = timer.now();
        self.check_time(&mut state, now)?;
        binding.require_time(now)?;
        if binding.run() != &self.run
            || binding.target() != &self.target
            || binding.authentication() != &self.authentication
            || state.seen.contains(&binding.attempt())
            || matches!(state.phase, SessionPhase::Retired | SessionPhase::Live(_))
        {
            return Err(CoordinatorError::Conflict);
        }
        if state.seen.len() >= nocterm_cloud_model::login::MAX_LOGIN_CONSENTS {
            return Err(CoordinatorError::Overloaded);
        }
        Ok(())
    }
    pub(super) fn established_durably(
        &self,
        ticket: &SessionTicket,
        fact: Arc<Allocated<SessionObservation>>,
    ) -> Result<(), CoordinatorError> {
        if fact.value().binding() != ticket.binding()
            || !matches!(fact.value().outcome(), SessionOutcome::Established(_))
        {
            return Err(CoordinatorError::Conflict);
        }
        let mut state = self.state.lock();
        self.require_current(&state, ticket)?;
        if !matches!(state.phase, SessionPhase::Pending(_)) {
            return Err(CoordinatorError::Conflict);
        }
        let SessionPhase::Pending(active) =
            std::mem::replace(&mut state.phase, SessionPhase::Empty)
        else {
            return Err(CoordinatorError::Conflict);
        };
        state.phase = SessionPhase::Durable(active, fact);
        self.changed.send_replace(());
        Ok(())
    }
    pub(super) fn activate(
        &self,
        ticket: &SessionTicket,
        publication: &dyn PublishedRunSession,
        fence: &SessionActivationFence,
    ) -> Result<(), CoordinatorError> {
        let mut state = self.state.lock();
        self.require_current(&state, ticket)?;
        let SessionPhase::Durable(_, fact) = &state.phase else {
            return Err(CoordinatorError::Conflict);
        };
        let SessionOutcome::Established(receipt) = fact.value().outcome() else {
            return Err(CoordinatorError::Conflict);
        };
        if publication.receipt() != receipt {
            return Err(CoordinatorError::Conflict);
        }
        // No await or physical/network callback while holding this cutoff. Stop
        // retires through the same state lock; late candidates cannot mark Live.
        let now = fence.check()?;
        self.check_time(&mut state, now)?;
        ticket.binding().require_time(now)?;
        let SessionPhase::Durable(_, fact) = &state.phase else {
            return Err(CoordinatorError::Conflict);
        };
        state.phase = SessionPhase::Live(LiveSession {
            generation: ticket.generation.clone(),
            fact: fact.clone(),
        });
        self.wake.store(true, Ordering::Release);
        self.changed.send_replace(());
        Ok(())
    }
    pub(super) fn live(&self) -> Option<Arc<Allocated<SessionObservation>>> {
        match &self.state.lock().phase {
            SessionPhase::Live(live) => Some(live.fact.clone()),
            _ => None,
        }
    }
    pub(super) fn permits_initial_reference(&self) -> bool {
        matches!(self.state.lock().phase, SessionPhase::Empty)
    }
    pub(super) fn restore_lost(&self, binding: &SessionAttempt) -> Result<(), CoordinatorError> {
        if binding.run() != &self.run
            || binding.target() != &self.target
            || binding.authentication() != &self.authentication
        {
            return Err(CoordinatorError::Conflict);
        }
        let mut state = self.state.lock();
        if matches!(state.phase, SessionPhase::Retired) {
            return Err(CoordinatorError::Stopped);
        }
        if !state.seen.contains(&binding.attempt())
            && state.seen.len() >= nocterm_cloud_model::login::MAX_LOGIN_CONSENTS
        {
            return Err(CoordinatorError::Overloaded);
        }
        state.seen.insert(binding.attempt());
        state.phase = SessionPhase::Lost;
        self.changed.send_replace(());
        Ok(())
    }
    pub(super) fn abandon(&self, ticket: &SessionTicket) {
        let mut state = self.state.lock();
        if self.require_current(&state, ticket).is_ok()
            && !matches!(state.phase, SessionPhase::Live(_))
        {
            state.phase = SessionPhase::Lost;
            self.wake.store(true, Ordering::Release);
            self.changed.send_replace(());
        }
    }
    pub(super) fn observe_loss(
        &self,
        ticket: &SessionTicket,
        observation: &SessionObservation,
    ) -> Result<(), CoordinatorError> {
        if observation.binding() != ticket.binding() {
            return Err(CoordinatorError::Conflict);
        }
        let SessionOutcome::Lost { session, .. } = observation.outcome() else {
            return Err(CoordinatorError::InvalidCommand);
        };
        let mut state = self.state.lock();
        if self.require_current(&state, ticket).is_err() {
            // A late original fact may be journaled by cleanup, but cannot fence
            // a replacement attempt or change that replacement's selected state.
            return Ok(());
        }
        let fact = match &state.phase {
            SessionPhase::Durable(_, fact) => fact,
            SessionPhase::Live(live) => &live.fact,
            _ => return Err(CoordinatorError::Conflict),
        };
        if !matches!(fact.value().outcome(), SessionOutcome::Established(receipt) if receipt == session)
        {
            return Err(CoordinatorError::Conflict);
        }
        state.phase = SessionPhase::Lost;
        self.wake.store(true, Ordering::Release);
        self.changed.send_replace(());
        Ok(())
    }
    pub(super) fn retire(&self) {
        self.state.lock().phase = SessionPhase::Retired;
        self.changed.send_replace(());
    }
    pub(super) fn cancel_pending(&self) {
        let mut state = self.state.lock();
        if matches!(
            state.phase,
            SessionPhase::Pending(_) | SessionPhase::Durable(..)
        ) {
            let phase = std::mem::replace(&mut state.phase, SessionPhase::Lost);
            self.changed.send_replace(());
            drop(state);
            if let SessionPhase::Pending(active) | SessionPhase::Durable(active, _) = phase
                && let Some(cancellation) = active.cancellation
            {
                cancellation.cancel();
            }
        }
    }
    fn require_current(
        &self,
        state: &SessionState,
        ticket: &SessionTicket,
    ) -> Result<(), CoordinatorError> {
        if state.phase.active().is_some_and(|(generation, binding)| {
            Arc::ptr_eq(generation, &ticket.generation) && binding == ticket.binding()
        }) {
            Ok(())
        } else {
            Err(CoordinatorError::Stopped)
        }
    }
    fn check_time(&self, state: &mut SessionState, now: Timestamp) -> Result<(), CoordinatorError> {
        if state.observed.is_some_and(|old| now < old) {
            state.phase = SessionPhase::Retired;
            self.changed.send_replace(());
            return Err(CoordinatorError::Expired);
        }
        state.observed = Some(now);
        Ok(())
    }
}

/// Four bounded namespace strings plus fixed coordinates, tree node, Arc and
/// phase storage. Caller mints this from its finite session metadata pool before
/// cloning; it is separate from the short-lived command ingress/work permits.
pub(super) fn session_descriptor_bytes() -> usize {
    4096 + 16 * nocterm_cloud_model::identity::MAX_NAMESPACE_BYTES
}

/// Actual original source, run stop owner and current writer, never DTO checks.
#[derive(Clone)]
pub(super) struct SessionActivationFence {
    source: Arc<dyn AuthenticatedCommandSource>,
    control: Arc<RunControl>,
    log: Arc<ChatLog>,
    timer: Arc<dyn ToolTimer>,
}
impl SessionActivationFence {
    pub(super) fn new(
        source: Arc<dyn AuthenticatedCommandSource>,
        control: Arc<RunControl>,
        log: Arc<ChatLog>,
        timer: Arc<dyn ToolTimer>,
    ) -> Self {
        Self {
            source,
            control,
            log,
            timer,
        }
    }
    pub(super) fn check(&self) -> Result<Timestamp, CoordinatorError> {
        self.control.check(self.source.as_ref())?;
        self.log.require_current()?;
        self.control.check(self.source.as_ref())?;
        Ok(self.timer.now())
    }
    pub(super) fn check_connection(&self) -> Result<(), CoordinatorError> {
        self.source.check_current()
    }
    pub(super) fn check_pending(&self, ticket: &SessionTicket) -> Result<(), CoordinatorError> {
        self.control.check(self.source.as_ref())?;
        self.log.require_current()?;
        ticket.check_pending(self.timer.as_ref())
    }
    pub(super) fn require_connection(
        &self,
        ticket: &SessionTicket,
    ) -> Result<(), CoordinatorError> {
        self.check_connection()?;
        if ticket.needs_run_control()? {
            self.control.check(self.source.as_ref())?;
            self.log.require_current()?;
        }
        ticket.check_connection(self.timer.as_ref())?;
        self.source.check_current()
    }
    pub(super) fn ended(&self, ticket: SessionTicket) -> futures::future::BoxFuture<'static, ()> {
        let fence = self.clone();
        Box::pin(async move {
            loop {
                if fence.require_connection(&ticket).is_err() {
                    return;
                }
                tokio::select! {
                    _ = ticket.ended() => return,
                    _ = fence.timer.delay(std::time::Duration::from_millis(100)) => {}
                }
            }
        })
    }
}
