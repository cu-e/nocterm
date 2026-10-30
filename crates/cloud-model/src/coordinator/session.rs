//! Finite initial connection requests and descriptive historical session facts.
//! These values neither authenticate their issuer nor publish a native session.
use super::CoordinatorError;
use crate::{
    authority::{AuthenticationBinding, RunBinding, SessionReceipt, TargetBinding},
    grant::{Capabilities, GrantWindow},
    identity::{AttemptId, Fingerprint, Timestamp},
    login::{LoginConsentBinding, MAX_LOGIN_CONSENT_MS},
};
use nocterm_ai::ToolCapability;

/// One original device-issued attempt. Local startup needs no SSH key or receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAttempt {
    run: RunBinding,
    target: TargetBinding,
    authentication: AuthenticationBinding,
    attempt: AttemptId,
    nonce: Fingerprint,
    window: GrantWindow,
}
impl SessionAttempt {
    pub fn new(
        run: RunBinding,
        target: TargetBinding,
        authentication: AuthenticationBinding,
        attempt: AttemptId,
        nonce: Fingerprint,
        window: GrantWindow,
    ) -> Result<Self, CoordinatorError> {
        if window.issued().as_millis() == 0
            || window.expires().as_millis() > i64::MAX as u64
            || window
                .expires()
                .elapsed_since(window.issued())
                .is_none_or(|duration| duration == 0 || duration > MAX_LOGIN_CONSENT_MS)
            || nonce.as_bytes() == &[0; 32]
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            run,
            target,
            authentication,
            attempt,
            nonce,
            window,
        })
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn target(&self) -> &TargetBinding {
        &self.target
    }
    pub fn authentication(&self) -> &AuthenticationBinding {
        &self.authentication
    }
    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }
    pub fn nonce(&self) -> Fingerprint {
        self.nonce
    }
    pub fn window(&self) -> GrantWindow {
        self.window
    }
    pub fn require_time(&self, now: Timestamp) -> Result<(), CoordinatorError> {
        if now < self.window.issued() || now >= self.window.expires() {
            return Err(CoordinatorError::Expired);
        }
        Ok(())
    }
    pub fn matches_receipt(&self, receipt: &SessionReceipt) -> bool {
        receipt.target() == &self.target && receipt.authentication() == &self.authentication
    }
}

/// Connection-only transports must refuse this request if they always start PTY.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStartup {
    ConnectOnly,
    Terminal,
}
impl SessionStartup {
    /// Descriptive requested capabilities, never a tool grant or execution permit.
    pub fn capabilities(self) -> Capabilities {
        let connection = Capabilities::NONE.with(ToolCapability::Connection);
        match self {
            Self::ConnectOnly => connection,
            Self::Terminal => connection.with(ToolCapability::Execution),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRequest {
    binding: SessionAttempt,
    startup: SessionStartup,
    ssh: Option<LoginConsentBinding>,
}
impl SessionRequest {
    pub fn local(binding: SessionAttempt, startup: SessionStartup) -> Self {
        Self {
            binding,
            startup,
            ssh: None,
        }
    }
    /// Reuses the exact checked SSH consent, including the actual public key.
    pub fn ssh(
        consent: LoginConsentBinding,
        startup: SessionStartup,
    ) -> Result<Self, CoordinatorError> {
        let binding = SessionAttempt::new(
            consent.run().clone(),
            consent.target().clone(),
            consent.authentication().clone(),
            consent.attempt(),
            Fingerprint::from_bytes(*consent.nonce()),
            GrantWindow::new(consent.issued(), consent.expires())?,
        )?;
        Ok(Self {
            binding,
            startup,
            ssh: Some(consent),
        })
    }
    pub fn binding(&self) -> &SessionAttempt {
        &self.binding
    }
    pub fn startup(&self) -> SessionStartup {
        self.startup
    }
    pub fn ssh_consent(&self) -> Option<&LoginConsentBinding> {
        self.ssh.as_ref()
    }
    /// Native format reconstruction reuses the same checked public-key and
    /// one-use consent constructors; this never creates device approval.
    pub fn ssh_key(
        binding: SessionAttempt,
        startup: SessionStartup,
        public_key: &str,
    ) -> Result<Self, CoordinatorError> {
        let backend = |error| CoordinatorError::Backend(nocterm_core::SharedCause::new(error));
        if public_key.len() > 2048 {
            return Err(CoordinatorError::InvalidCommand);
        }
        let key = nocterm_ssh_keys::PublicKeyText::parse(public_key).map_err(backend)?;
        let window = crate::login::LoginConsentWindow::new(
            binding.window().issued(),
            binding.window().expires(),
        )
        .map_err(|error| CoordinatorError::Backend(nocterm_core::SharedCause::new(error)))?;
        let consent = LoginConsentBinding::new(
            binding.run().clone(),
            binding.target().clone(),
            binding.authentication().clone(),
            binding.attempt(),
            *binding.nonce().as_bytes(),
            window,
            key,
        )
        .map_err(|error| CoordinatorError::Backend(nocterm_core::SharedCause::new(error)))?;
        Self::ssh(consent, startup)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionFailure {
    Denied,
    Expired,
    Cancelled,
    SourceChanged,
    Unavailable,
    Unsupported,
    Capacity,
    ConnectionLost,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionOutcome {
    Established(SessionReceipt),
    Lost {
        session: SessionReceipt,
        reason: SessionFailure,
    },
    Refused(SessionFailure),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionObservation {
    binding: SessionAttempt,
    outcome: SessionOutcome,
}
impl SessionObservation {
    pub fn new(binding: SessionAttempt, outcome: SessionOutcome) -> Result<Self, CoordinatorError> {
        let receipt = match &outcome {
            SessionOutcome::Established(receipt)
            | SessionOutcome::Lost {
                session: receipt, ..
            } => Some(receipt),
            SessionOutcome::Refused(_) => None,
        };
        if receipt.is_some_and(|receipt| !binding.matches_receipt(receipt)) {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self { binding, outcome })
    }
    pub fn binding(&self) -> &SessionAttempt {
        &self.binding
    }
    pub fn outcome(&self) -> &SessionOutcome {
        &self.outcome
    }
}

/// Injected actual physical publication, never a capability made from a receipt.
/// Preparation has already observed Connected through the sole native reader.
/// The caller durably records Established before calling `publish`.
pub trait PreparedRunSession: Send {
    fn receipt(&self) -> &SessionReceipt;
    fn publish(self: Box<Self>) -> Result<Box<dyn PublishedRunSession>, CoordinatorError>;
}
/// Actual post-publication conditional retirement. Until `accept`, dropping this
/// owner retires ONLY its original physical generation, even after cancellation.
/// Acceptance follows source/writer/generation checks and original-run Live+wake
/// in one synchronous transition. The retained supervisor then owns the session.
pub trait PublishedRunSession: Send {
    fn receipt(&self) -> &SessionReceipt;
    fn accept(self: Box<Self>);
}

/// Actual host broker, injected once into the original coordinator. Admission
/// queues bounded tracked work and returns before DNS/SSH/PTY waits. Its answer
/// consumer transfers the exact admitted one-use proof; the live receipt arrives
/// only through the original fact sink after actual Connected.
pub trait RunSessionBroker: super::SessionConsentConsumer {
    /// Borrow-check exact pending proof/issuer before durable Answer admission;
    /// this never consumes the staged signature or releases a handshake.
    fn check_answer(
        &self,
        command: &super::CommandAcknowledgement,
        answer: &super::SessionConsent,
    ) -> Result<(), CoordinatorError>;
    fn begin(
        &self,
        command: super::CommandAcknowledgement,
        request: std::sync::Arc<nocterm_core::allocation::Allocated<SessionRequest>>,
        facts: std::sync::Arc<dyn SessionFactSink>,
    ) -> Result<Box<dyn SessionAttemptCancellation>, CoordinatorError>;
    /// Explicit chat removal retires that chat's actual retained supervisor.
    /// Foreground Stop/Completed only cancel pending login and detach run views.
    fn close_chat(&self, stream: &crate::authority::StreamBinding);
}
/// Opaque original pending ownership. Cancellation must compare actual private
/// generation identity and must not close an accepted retained physical session.
pub trait SessionAttemptCancellation: Send + Sync {
    fn cancel(&self);
}
/// One immutable original run/attempt. Implemented by the native coordinator,
/// not by the transport, public DTO mapper or model.
pub trait SessionFactSink: Send + Sync {
    fn check_current(&self) -> Result<(), CoordinatorError>;
    fn ended(&self) -> futures::future::BoxFuture<'static, ()>;
    fn authentication_requested(
        &self,
        request: std::sync::Arc<
            nocterm_core::allocation::Allocated<crate::login::LoginProofRequest>,
        >,
    ) -> futures::future::BoxFuture<'static, Result<(), CoordinatorError>>;
    /// Commits Established, then publishes the actual candidate, then installs
    /// Live+wakes the original run. A durable description alone never wakes it.
    fn established(
        &self,
        candidate: Box<dyn PreparedRunSession>,
    ) -> futures::future::BoxFuture<'static, Result<(), CoordinatorError>>;
    /// Observing loss immediately fences the original pending/live generation;
    /// its fact may commit later through current-writer fact-only cleanup.
    fn lost(
        &self,
        observation: std::sync::Arc<nocterm_core::allocation::Allocated<SessionObservation>>,
    ) -> futures::future::BoxFuture<'static, Result<(), CoordinatorError>>;
    fn refused(
        &self,
        reason: SessionFailure,
    ) -> futures::future::BoxFuture<'static, Result<(), CoordinatorError>>;
}
