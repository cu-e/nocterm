//! One pending initial SSH exchange; no generic signer or private key custody.
use nocterm_cloud_model::{
    coordinator::{
        CommandAcknowledgement, CoordinatorError, SessionConsent, SessionFactSink, SessionRequest,
    },
    journal::{PublicationAllocation, PublicationWorkBudget},
    login::{LoginError, LoginProofRequest, SessionLoginAuthorizer},
    tool::ToolTimer,
};
use nocterm_core::{
    SharedCause,
    allocation::{Allocated, AllocationLease},
};
use nocterm_hub_io::cloud::{RunnerCommandAttachments, RunnerSshAnswer};
use nocterm_ssh_keys::login::LoginSignature;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::oneshot;

struct Identity {
    original: CommandAcknowledgement,
    request: Arc<Allocated<SessionRequest>>,
}
enum Exchange {
    Fresh,
    Waiting {
        request: Arc<Allocated<LoginProofRequest>>,
        sender: Option<oneshot::Sender<RunnerSshAnswer>>,
    },
    Answered {
        _signature: Arc<Allocated<Option<LoginSignature>>>,
    },
    Closed,
}
#[derive(Clone)]
pub(crate) struct SessionProofExchange {
    identity: Arc<Allocated<Identity>>,
    facts: Arc<dyn SessionFactSink>,
    attachments: Arc<RunnerCommandAttachments>,
    timer: Arc<dyn ToolTimer>,
    budget: Arc<dyn PublicationWorkBudget>,
    state: Arc<Mutex<Exchange>>,
}
impl SessionProofExchange {
    pub(crate) fn new(
        original: CommandAcknowledgement,
        request: Arc<Allocated<SessionRequest>>,
        facts: Arc<dyn SessionFactSink>,
        attachments: Arc<RunnerCommandAttachments>,
        timer: Arc<dyn ToolTimer>,
        budget: Arc<dyn PublicationWorkBudget>,
    ) -> Result<Self, CoordinatorError> {
        if request.value().ssh_consent().is_none()
            || request.value().binding().run().stream() != *original.stream()
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        let allocation = budget.try_reserve(PublicationAllocation::new(16 * 1024, 1)?)?;
        Ok(Self {
            identity: Arc::new(Allocated::new(Identity { original, request }, allocation)),
            facts,
            attachments,
            timer,
            budget,
            state: Arc::new(Mutex::new(Exchange::Fresh)),
        })
    }
    pub(crate) fn check_answer(
        &self,
        command: &CommandAcknowledgement,
        answer: &SessionConsent,
    ) -> Result<(), CoordinatorError> {
        self.facts.check_current()?;
        let original = self.identity.value();
        let binding = original.request.value().binding();
        if command.stream() != original.original.stream()
            || command.acceptance().issuer() != original.original.acceptance().issuer()
            || answer.run() != binding.run()
            || answer.attempt() != binding.attempt()
            || answer.nonce() != binding.nonce()
            || answer.target() != binding.target()
            || answer.authentication() != binding.authentication()
            || answer.session().is_some()
            || answer.expires() != binding.window().expires()
            || answer.algorithm().as_str() != "ssh-ed25519"
        {
            return Err(CoordinatorError::Conflict);
        }
        binding.require_time(self.timer.now())?;
        let state = self
            .state
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        let Exchange::Waiting {
            request,
            sender: Some(_),
        } = &*state
        else {
            return Err(CoordinatorError::Conflict);
        };
        self.attachments.check_ssh(command, answer, request.value())
    }
    /// Called only after the original Answer command's durable admission.
    pub(crate) fn deliver(
        &self,
        command: &CommandAcknowledgement,
        answer: &SessionConsent,
    ) -> Result<(), CoordinatorError> {
        self.check_answer(command, answer)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| CoordinatorError::NativeTaskFailed)?;
        let Exchange::Waiting { sender, .. } = &mut *state else {
            return Err(CoordinatorError::Conflict);
        };
        let value = self
            .attachments
            .take_ssh(command, answer)?
            .ok_or(CoordinatorError::Conflict)?;
        sender
            .take()
            .ok_or(CoordinatorError::Conflict)?
            .send(value)
            .map_err(|_| CoordinatorError::Stopped)
    }
    async fn authorize(&self, request: LoginProofRequest) -> Result<LoginSignature, LoginError> {
        self.facts.check_current().map_err(credential)?;
        if self.identity.value().request.value().ssh_consent() != Some(request.binding()) {
            return Err(LoginError::Mismatch);
        }
        let allocation = self.reserve_request(&request)?;
        let request = Arc::new(Allocated::new(request, allocation));
        let (send, receive) = oneshot::channel();
        {
            let mut state = self.state.lock().map_err(|_| LoginError::NeedsUnlock)?;
            if !matches!(*state, Exchange::Fresh) {
                return Err(LoginError::Replay);
            }
            *state = Exchange::Waiting {
                request: request.clone(),
                sender: Some(send),
            };
        }
        let completion = ProofWait {
            state: self.state.clone(),
            request: request.clone(),
        };
        self.facts
            .authentication_requested(request.clone())
            .await
            .map_err(credential)?;
        let expires = request.value().binding().expires();
        let now = self.timer.now();
        if now >= expires || now < request.value().binding().issued() {
            return Err(LoginError::Expired);
        }
        let answer = tokio::select! {
            result = receive => result.map_err(|_| LoginError::NeedsUnlock)?,
            _ = self.facts.ended() => return Err(LoginError::Revoked),
            _ = self.timer.delay(Duration::from_millis(expires.as_millis() - now.as_millis())) => return Err(LoginError::Expired),
        };
        self.facts.check_current().map_err(credential)?;
        let answer = Arc::new(answer.answer(request.value())?);
        let signature = answer.value().as_ref().ok_or(LoginError::NeedsUnlock)?;
        signature.verify(request.value().login())?;
        let signature = signature.clone();
        self.facts.check_current().map_err(credential)?;
        *self.state.lock().map_err(|_| LoginError::NeedsUnlock)? =
            Exchange::Answered { _signature: answer };
        // The actual remote authorizer remains in SessionLoginAttempt until the
        // existing SSH path finishes authentication, then drops it before rekey.
        drop(completion);
        Ok(signature)
    }
    fn reserve_request(&self, request: &LoginProofRequest) -> Result<AllocationLease, LoginError> {
        let lengths = [
            (request.host().host_key_blob().len(), 1024),
            (request.host().signature_blob().len(), 1024),
            (request.login().payload().len(), 4096),
            (request.binding().user_key().as_str().len(), 2048),
        ];
        let bytes = lengths
            .into_iter()
            .try_fold(16 * 1024usize, |total, (length, maximum)| {
                if length > maximum {
                    return Err(LoginError::InvalidConsent);
                }
                total.checked_add(2 * length).ok_or(LoginError::Capacity)
            })?;
        self.budget
            .try_reserve(PublicationAllocation::new(bytes, 1).map_err(credential)?)
            .map_err(credential)
    }
}
impl SessionLoginAuthorizer for SessionProofExchange {
    fn authorize_login(
        &self,
        request: LoginProofRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LoginSignature, LoginError>> + Send + 'static>> {
        let this = self.clone();
        Box::pin(async move { this.authorize(request).await })
    }
}
struct ProofWait {
    state: Arc<Mutex<Exchange>>,
    request: Arc<Allocated<LoginProofRequest>>,
}
impl Drop for ProofWait {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock()
            && matches!(&*state, Exchange::Waiting { request, .. } if Arc::ptr_eq(request, &self.request))
        {
            *state = Exchange::Closed;
        }
    }
}
fn credential(error: impl std::error::Error + Send + Sync + 'static) -> LoginError {
    LoginError::Credential(SharedCause::new(error))
}
