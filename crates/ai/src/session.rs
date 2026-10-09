//! A chat's agent session as a pure state machine.
//!
//! A chat owns one [`SessionState`]. Every change to its session goes through
//! one of the transitions below, so states such as "generating without a
//! session" or "failed but still accepting updates" cannot be built. The
//! transitions only decide; the chat performs the side effects they ask for
//! and drops async results whose [`Ticket`] is no longer current.
use crate::AgentError;

/// Where a chat's agent session is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionPhase {
    /// No agent connection. The next prompt or warm start connects.
    #[default]
    Detached,
    /// Waiting for the runtime to admit a connection.
    Queued,
    /// The agent process is starting or the session is being opened.
    Opening,
    /// The session is open and has no turn running.
    Ready,
    /// A turn is running.
    Prompting,
    /// The user stopped the running turn; its late updates are dropped.
    Cancelling,
    /// The agent needs the user to sign in.
    SignInRequired,
    /// Signing in.
    SigningIn,
    /// The session ended with an error. The next prompt reconnects.
    Failed,
}

/// Identifies one async request against a session; a result whose ticket is
/// stale belongs to a session or turn that was replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ticket {
    epoch: u64,
    turn: u64,
}

impl Ticket {
    pub fn epoch(self) -> u64 {
        self.epoch
    }
    pub fn turn(self) -> u64 {
        self.turn
    }
}

/// Consecutive failures after which a chat stops reopening the same agent
/// session and starts a new one with the saved conversation.
const RESUME_ATTEMPTS: u32 = 1;

#[derive(Clone, Debug, Default)]
pub struct SessionState {
    phase: SessionPhase,
    /// Advances whenever the session is replaced.
    epoch: u64,
    /// Advances with every turn.
    turn: u64,
    /// The user stopped the chat since the last turn started.
    stopped: bool,
    /// A turn was cancelled: its late updates are dropped until the next one.
    discarding: bool,
    /// The previous connection is still closing.
    closing: bool,
    /// Failures since the last completed turn.
    failures: u32,
}

impl SessionState {
    pub fn phase(&self) -> SessionPhase {
        self.phase
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn turn(&self) -> u64 {
        self.turn
    }
    /// A ticket for a request against the current session.
    pub fn ticket(&self) -> Ticket {
        Ticket {
            epoch: self.epoch,
            turn: self.turn,
        }
    }
    /// Whether a request issued with `ticket` still targets the session.
    pub fn session_current(&self, ticket: Ticket) -> bool {
        self.epoch == ticket.epoch
    }
    /// Whether a turn issued with `ticket` is still the running one.
    pub fn turn_current(&self, ticket: Ticket) -> bool {
        self.epoch == ticket.epoch && self.turn == ticket.turn
    }

    /// A turn is running, including one being stopped.
    pub fn generating(&self) -> bool {
        matches!(
            self.phase,
            SessionPhase::Prompting | SessionPhase::Cancelling
        )
    }
    /// Whether session updates still change the transcript.
    pub fn accepts_updates(&self) -> bool {
        !self.discarding && self.phase != SessionPhase::Failed
    }
    /// The session ended with an error; the next prompt reconnects.
    pub fn failed(&self) -> bool {
        self.phase == SessionPhase::Failed
    }
    pub fn stopped(&self) -> bool {
        self.stopped
    }
    /// The chat waits for the runtime to admit its connection.
    pub fn queued(&self) -> bool {
        self.phase == SessionPhase::Queued
    }
    /// The connection or session is being opened.
    pub fn opening(&self) -> bool {
        self.phase == SessionPhase::Opening
    }
    /// Sending waits until the user signs in.
    pub fn sign_in_required(&self) -> bool {
        matches!(
            self.phase,
            SessionPhase::SignInRequired | SessionPhase::SigningIn
        )
    }
    pub fn signing_in(&self) -> bool {
        self.phase == SessionPhase::SigningIn
    }
    /// The previous connection is still closing.
    pub fn closing(&self) -> bool {
        self.closing
    }
    /// Whether closing the session now would interrupt work.
    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            SessionPhase::Opening
                | SessionPhase::Prompting
                | SessionPhase::Cancelling
                | SessionPhase::SignInRequired
                | SessionPhase::SigningIn
        )
    }
    /// Whether reconnecting may reopen the same agent session. After
    /// repeated failures the chat starts a new session with its saved
    /// conversation instead, so one broken session cannot wedge it.
    pub fn may_resume(&self) -> bool {
        self.failures <= RESUME_ATTEMPTS
    }

    /// Asks for a connection. Returns whether the runtime must be asked:
    /// only a detached or failed chat connects, and a failed one recovers.
    pub fn queue(&mut self) -> bool {
        match self.phase {
            SessionPhase::Detached | SessionPhase::Failed => {
                self.phase = SessionPhase::Queued;
                self.stopped = false;
                self.discarding = false;
                true
            }
            _ => false,
        }
    }
    /// Gives up a request the runtime has not admitted yet. Returns whether
    /// there was one.
    pub fn withdraw(&mut self) -> bool {
        if self.phase != SessionPhase::Queued {
            return false;
        }
        self.phase = SessionPhase::Detached;
        true
    }
    /// The runtime admitted the chat and started its connection.
    pub fn leased(&mut self) {
        self.phase = SessionPhase::Opening;
    }
    /// The session is open.
    pub fn opened(&mut self) {
        if self.phase == SessionPhase::Opening {
            self.phase = SessionPhase::Ready;
        }
    }
    /// Starts a turn; `None` unless the session is ready for one.
    pub fn start_turn(&mut self) -> Option<Ticket> {
        if self.phase != SessionPhase::Ready {
            return None;
        }
        self.phase = SessionPhase::Prompting;
        self.stopped = false;
        self.discarding = false;
        self.turn += 1;
        Some(self.ticket())
    }
    /// The user stops the chat. Returns whether a running turn must be
    /// cancelled.
    pub fn stop(&mut self) -> bool {
        self.stopped = true;
        if !self.generating() {
            return false;
        }
        self.phase = SessionPhase::Cancelling;
        self.discarding = true;
        true
    }
    /// The turn of `ticket` finished. Returns whether it was the running
    /// one; a stale result changes nothing.
    pub fn finish_turn(&mut self, ticket: Ticket, completed: bool) -> bool {
        if !self.turn_current(ticket) || !self.generating() {
            return false;
        }
        self.phase = SessionPhase::Ready;
        if completed {
            self.failures = 0;
        }
        true
    }
    /// The agent asks the user to sign in.
    pub fn require_sign_in(&mut self) {
        self.phase = SessionPhase::SignInRequired;
    }
    /// Returns whether signing in may start.
    pub fn begin_sign_in(&mut self) -> bool {
        if self.phase != SessionPhase::SignInRequired {
            return false;
        }
        self.phase = SessionPhase::SigningIn;
        true
    }
    /// Signing in finished. With a session open the chat is ready again;
    /// without one it goes on opening it.
    pub fn sign_in_finished(&mut self, succeeded: bool, has_session: bool) {
        if self.phase != SessionPhase::SigningIn {
            return;
        }
        self.phase = match (succeeded, has_session) {
            (false, _) => SessionPhase::SignInRequired,
            (true, true) => SessionPhase::Ready,
            (true, false) => SessionPhase::Opening,
        };
    }
    /// The session ended with an error. Results of its requests are dropped.
    pub fn fail(&mut self) {
        self.phase = SessionPhase::Failed;
        self.epoch += 1;
        self.stopped = false;
        self.discarding = false;
        self.failures = self.failures.saturating_add(1);
    }
    /// The chat let its connection go. A failed chat stays failed, so the
    /// user still sees why; otherwise it is detached until needed again.
    pub fn detach(&mut self) {
        self.epoch += 1;
        self.closing = true;
        if self.phase != SessionPhase::Failed {
            self.phase = SessionPhase::Detached;
        }
    }
    /// The chat released everything it held, a failure included.
    pub fn release(&mut self) {
        self.phase = SessionPhase::Detached;
        self.stopped = false;
        self.discarding = false;
    }
    /// The previous connection finished closing.
    pub fn closed(&mut self) {
        self.closing = false;
    }
    /// Puts the session in `phase` without a transition, for tests of what
    /// observes it.
    #[cfg(feature = "test-support")]
    pub fn force_phase(&mut self, phase: SessionPhase) {
        self.phase = phase;
    }
}

/// What to do after reopening a saved agent session failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestoreFallback {
    /// Try the same request again.
    Retry,
    /// Open a new session; the saved conversation goes with the next prompt.
    StartFresh,
    /// The agent needs the user to sign in; reopen afterwards.
    SignIn,
    /// Give up for now and keep the session to reopen on the next message.
    Fail,
}

/// How many times reopening a session is tried before starting a new one.
pub const RESTORE_ATTEMPTS: u32 = 2;

/// Decides what follows a failed resume, load or fork.
///
/// A session the agent no longer has is replaced by a new one that receives
/// the saved conversation. A fork has no agent state of its own yet, so it
/// starts over after any error. A chat's own session may hold more than its
/// transcript, so a transient error does not discard it: the chat fails, the
/// next message tries again, and [`SessionState::may_resume`] gives up on the
/// session only after repeated failures.
pub fn restore_fallback(error: &AgentError, attempt: u32, fork: bool) -> RestoreFallback {
    match error {
        AgentError::AuthRequired(_) => RestoreFallback::SignIn,
        AgentError::RestoreUnavailable(_) => RestoreFallback::StartFresh,
        _ if attempt + 1 < RESTORE_ATTEMPTS => RestoreFallback::Retry,
        _ if fork => RestoreFallback::StartFresh,
        _ => RestoreFallback::Fail,
    }
}

#[cfg(test)]
mod tests;
