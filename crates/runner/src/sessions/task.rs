//! One tracked driver and lifecycle consumer for an actual original connection.
use super::{
    login::SessionProofExchange,
    publication::{PendingConnection, PhysicalCandidate},
    resources::SessionAdmission,
    setup::SessionBrokerSetup,
    source::SessionConnectionScope,
    targets::ConfiguredSessionTargets,
};
use nocterm_agent_tools::headless::{
    InitialSessionCandidate, RetainedTerminals, SessionLifecycleEvent, SessionObservations,
};
use nocterm_cloud_model::{
    authority::SessionReceipt, coordinator::*, identity::*, journal::PublicationAllocation,
    login::SessionLoginAuthorizer,
};
use nocterm_core::{SharedCause, allocation::Allocated};
use nocterm_session::Session;
use std::sync::{Arc, Mutex};

pub(super) struct SessionAttemptWork {
    request: Arc<Allocated<SessionRequest>>,
    facts: Arc<dyn SessionFactSink>,
    proof: Option<Arc<SessionProofExchange>>,
    pending: Arc<PendingConnection>,
    admission: SessionAdmission,
}
impl SessionAttemptWork {
    pub(super) fn new(
        request: Arc<Allocated<SessionRequest>>,
        facts: Arc<dyn SessionFactSink>,
        proof: Option<Arc<SessionProofExchange>>,
        pending: Arc<PendingConnection>,
        admission: SessionAdmission,
    ) -> Self {
        Self {
            request,
            facts,
            proof,
            pending,
            admission,
        }
    }
}
pub(super) struct SessionTask {
    setup: Arc<SessionBrokerSetup>,
    targets: Arc<ConfiguredSessionTargets>,
    retained: RetainedTerminals,
    attempt: SessionAttemptWork,
}
impl SessionTask {
    pub(super) fn new(
        setup: Arc<SessionBrokerSetup>,
        targets: Arc<ConfiguredSessionTargets>,
        retained: RetainedTerminals,
        attempt: SessionAttemptWork,
    ) -> Self {
        Self {
            setup,
            targets,
            retained,
            attempt,
        }
    }
    pub(super) async fn run(self) -> Result<(), CoordinatorError> {
        self.setup.source().check_current()?;
        self.attempt.facts.check_current()?;
        self.attempt.pending.check()?;
        let binding = self.attempt.request.value().binding();
        let receipt = SessionReceipt::new(
            RemoteSessionId::from_bytes(self.setup.identity().fresh()?),
            SessionEpoch::new(1)?,
            binding.target().clone(),
            binding.authentication().clone(),
        );
        let scope = SessionConnectionScope::new(
            self.setup.connection().clone(),
            binding.run(),
            self.attempt.facts.clone(),
            self.attempt.admission.residency(),
        )?;
        let authorizer = self
            .attempt
            .proof
            .as_ref()
            .map(|proof| proof.clone() as Arc<dyn SessionLoginAuthorizer>);
        let session = self.targets.open(
            self.attempt.request.value(),
            scope.source(),
            authorizer,
            self.attempt.admission.allocation(),
            self.setup.timer().now(),
        )?;
        let lifetime = NativeSessionLifetime {
            session: session.clone(),
            scope: scope.clone(),
        };
        let close = session.clone();
        let options = self
            .targets
            .terminal_options(self.attempt.request.value(), self.setup.timer().now())?;
        let registry = Arc::new(Mutex::new(
            self.retained.view(binding.run().clone()).map_err(backend)?,
        ));
        let (candidate, observations, driver) = InitialSessionCandidate::prepare(
            registry,
            RemoteSessionId::from_bytes(self.setup.identity().fresh()?),
            receipt,
            session,
            options,
            self.setup.timer().clone(),
            self.attempt.admission.residency(),
        )
        .map_err(backend)?;
        let lifecycle = SessionLifecycle {
            setup: self.setup,
            attempt: self.attempt,
            scope,
            observations,
            candidate: Some(candidate),
            close,
            retained: self.retained,
        };
        let (_, result) = futures::join!(driver.into_run(), lifecycle.run());
        drop(lifetime);
        result
    }
}
struct SessionLifecycle {
    setup: Arc<SessionBrokerSetup>,
    attempt: SessionAttemptWork,
    scope: Arc<SessionConnectionScope>,
    observations: SessionObservations,
    candidate: Option<InitialSessionCandidate>,
    close: Session,
    retained: RetainedTerminals,
}
impl SessionLifecycle {
    async fn run(mut self) -> Result<(), CoordinatorError> {
        let first = tokio::select! {
            first = self.observations.recv() => first,
            _ = self.attempt.pending.cancelled() => None,
            _ = self.scope.ended(self.setup.timer().clone()) => None,
        };
        let Some(first) = first else {
            self.scope.close();
            drop(self.candidate.take());
            return Ok(());
        };
        if !matches!(first.event(), SessionLifecycleEvent::Connected) {
            drop(self.candidate.take());
            return self
                .attempt
                .facts
                .refused(SessionFailure::ConnectionLost)
                .await;
        }
        let candidate = self.candidate.take().ok_or(CoordinatorError::Conflict)?;
        self.attempt.pending.check()?;
        self.scope.connected(&candidate, &first)?;
        // Authentication is complete. Rekey retains only the real transport,
        // not the one-use device proof or its pending answer owner.
        drop(self.attempt.proof.take());
        // The proof-bearing handshake no longer occupies its reserved slot.
        // Physical guards remain in the actual Session source and sole reader.
        let residency = self.attempt.admission.established();
        let parts = SessionLifecycleParts {
            pending: &self.attempt.pending,
            facts: &self.attempt.facts,
            scope: &self.scope,
            setup: &self.setup,
            close: &self.close,
        };
        let established = publish_established(&parts, candidate).await;
        if established.is_ok() {
            self.attempt.pending.connected()?;
        }
        let loss = tokio::select! {
            loss = self.observations.recv() => loss,
            _ = self.scope.ended(self.setup.timer().clone()) => {
                self.scope.close();
                self.close.close();
                // Closing the actual source ends this original transport;
                // no descriptive receipt lookup can close a replacement.
                self.observations.recv().await
            }
        };
        let result = if let Some(loss) = loss {
            if !matches!(loss.event(), SessionLifecycleEvent::Lost { .. }) {
                return Err(CoordinatorError::Conflict);
            }
            let allocation = self
                .setup
                .work()
                .try_reserve(PublicationAllocation::new(16 * 1024, 1)?)?;
            let observation = Arc::new(Allocated::new(
                SessionObservation::new(
                    self.attempt.request.value().binding().clone(),
                    SessionOutcome::Lost {
                        session: loss.receipt().clone(),
                        reason: SessionFailure::ConnectionLost,
                    },
                )?,
                allocation,
            ));
            // The sink fences synchronously at call time before durable cleanup.
            let recording = self.attempt.facts.lost(observation);
            self.retained.retire_observed(&loss);
            let recorded = recording.await;
            established.and(recorded)
        } else {
            established
        };
        drop(residency);
        result
    }
}
struct SessionLifecycleParts<'a> {
    pending: &'a Arc<PendingConnection>,
    facts: &'a Arc<dyn SessionFactSink>,
    scope: &'a Arc<SessionConnectionScope>,
    setup: &'a Arc<SessionBrokerSetup>,
    close: &'a Session,
}
async fn publish_established(
    lifecycle: &SessionLifecycleParts<'_>,
    candidate: InitialSessionCandidate,
) -> Result<(), CoordinatorError> {
    let (candidate, cancellation) = PhysicalCandidate::new(candidate);
    let established = lifecycle.facts.established(Box::new(candidate));
    tokio::pin!(established);
    let established = tokio::select! {
        result = &mut established => result,
        _ = lifecycle.pending.cancelled() => {
            cancellation.cancel();
            established.await
        },
        _ = lifecycle.scope.ended(lifecycle.setup.timer().clone()) => {
            cancellation.cancel();
            lifecycle.scope.close();
            lifecycle.close.close();
            established.await
        },
    };
    established
}
struct NativeSessionLifetime {
    session: Session,
    scope: Arc<SessionConnectionScope>,
}
impl Drop for NativeSessionLifetime {
    fn drop(&mut self) {
        self.scope.close();
        self.session.close();
    }
}
fn backend(error: impl std::error::Error + Send + Sync + 'static) -> CoordinatorError {
    CoordinatorError::Backend(SharedCause::new(error))
}
