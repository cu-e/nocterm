//! Durable session facts precede physical publication and original-run wake.
use super::{
    frame::Frame,
    log::ChatLog,
    run_session::{
        RunSessionOwner, SessionActivationFence, SessionTicket, session_descriptor_bytes,
    },
    services::CoordinatorStorage,
    session_fact::SessionFact,
    session_services::SessionPublicationResources,
};
use futures::future::BoxFuture;
use nocterm_cloud_model::{
    coordinator::*, identity::Fingerprint, journal::*, login::LoginProofRequest,
};
use nocterm_core::allocation::{Allocated, AllocationLease};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct SessionFactPublisher {
    storage: CoordinatorStorage,
    fence: SessionActivationFence,
    log: Arc<ChatLog>,
    owner: Arc<RunSessionOwner>,
    ticket: SessionTicket,
    resources: SessionPublicationResources,
    original: CommandReceipt,
    established: Arc<Mutex<Option<Arc<Allocated<SessionEstablishedAnchor>>>>>,
}
impl SessionFactPublisher {
    pub(super) fn new(
        storage: CoordinatorStorage,
        fence: SessionActivationFence,
        log: Arc<ChatLog>,
        owner: Arc<RunSessionOwner>,
        ticket: SessionTicket,
        resources: SessionPublicationResources,
        original: CommandReceipt,
    ) -> Self {
        Self {
            storage,
            fence,
            log,
            owner,
            ticket,
            resources,
            original,
            established: Arc::new(Mutex::new(None)),
        }
    }
    fn native_work(&self, cleanup: bool) -> Result<AllocationLease, CoordinatorError> {
        // Four simultaneous native body generations: frame JSON, copied native
        // Record payload, Postcard plaintext and ciphertext. Bounded coordinate
        // clones live alongside them. The public encoder admits its own buffers.
        let bytes = 4 * MAX_SESSION_FACT_BYTES + 4 * session_descriptor_bytes();
        let budget = if cleanup {
            self.storage.publication_work().cleanup()
        } else {
            self.storage.publication_work().urgent()
        };
        budget
            .try_reserve(PublicationAllocation::new(bytes, 1)?)
            .map_err(CoordinatorError::from)
    }
    async fn persist(
        &self,
        observation: &SessionObservation,
        work: AllocationLease,
    ) -> Result<(JournalPosition, Fingerprint), CoordinatorError> {
        let frame = Frame::Session(SessionFact::capture(observation, self.log.lease().epoch())?);
        if super::format::encoded_len(&frame)? > MAX_SESSION_FACT_BYTES {
            return Err(JournalError::Capacity.into());
        }
        let body = frame.bytes()?;
        let digest = Fingerprint::from_bytes(Sha256::digest(body.as_bytes()).into());
        let record = JournalRecord::new(
            RecordContext::Run(observation.binding().run().clone()),
            JournalRecordKind::RunStatus,
            body,
        )?;
        let event = CoordinatorEvent::SessionState(Box::new(observation.clone()));
        let outcome = self
            .log
            .append_after_command_resident(&self.original, vec![(record, Some(event))], work)
            .await?;
        let CommitOutcome::Committed(receipt) = outcome else {
            return Err(CoordinatorError::Conflict);
        };
        Ok((receipt.position(), digest))
    }
    async fn establish(
        &self,
        candidate: Box<dyn PreparedRunSession>,
    ) -> Result<(), CoordinatorError> {
        let completion = PendingCompletion {
            owner: self.owner.clone(),
            ticket: self.ticket.clone(),
        };
        self.fence.check_pending(&self.ticket)?;
        let allocation = self.resources.fact_metadata(true)?;
        let work = self.native_work(false)?;
        let fact = Arc::new(Allocated::new(
            SessionObservation::new(
                self.ticket.binding().clone(),
                SessionOutcome::Established(candidate.receipt().clone()),
            )?,
            allocation.clone(),
        ));
        let (position, digest) = self.persist(fact.value(), work).await?;
        let anchor = SessionEstablishedAnchor::new(
            self.log.lease().epoch(),
            position.records(),
            position.revision(),
            fact.value().clone(),
            digest,
        )?;
        *self.established.lock() = Some(Arc::new(Allocated::new(anchor, allocation)));
        self.owner.established_durably(&self.ticket, fact)?;
        self.fence.require_connection(&self.ticket)?;
        let published = candidate.publish()?;
        let activated = {
            // The actual physical rollback owner remains alive while waiting.
            // Registry retirement on failure happens only after this gate drops.
            let _disk = self.storage.io().acquire().await?;
            self.owner
                .activate(&self.ticket, published.as_ref(), &self.fence)
        };
        activated?;
        published.accept();
        drop(completion);
        Ok(())
    }
    async fn publish_loss(
        &self,
        fact: Arc<Allocated<SessionObservation>>,
    ) -> Result<(), CoordinatorError> {
        let original = self
            .established
            .lock()
            .clone()
            .ok_or(CoordinatorError::Conflict)?;
        let cleanup = self.resources.cleanup().clone();
        let allocation = self
            .storage
            .publication_work()
            .cleanup()
            .try_reserve(PublicationAllocation::new(cleanup.work_bytes(), 1)?)?;
        let stream = self.ticket.binding().run().stream();
        let lease = self
            .storage
            .io()
            .owned_writer(&stream)
            .ok_or(JournalError::Fenced)?;
        let disk = self
            .storage
            .io()
            .acquire_resident(allocation.clone())
            .await?;
        let head = self
            .storage
            .io()
            .locked(&disk, move |journal| journal.head(stream))
            .await?
            .ok_or(JournalError::NotFound)?;
        if head.lease() != &lease {
            return Err(JournalError::Fenced.into());
        }
        let next = head.position().next(1, 1, false)?;
        let native = Frame::Session(SessionFact::capture(fact.value(), lease.epoch())?).bytes()?;
        let event = CoordinatorEvent::SessionState(Box::new(fact.value().clone()));
        let outbox = self.storage.encoder().encode(
            &lease,
            next,
            &event,
            self.storage.publication_work().cleanup().as_ref(),
        )?;
        let batch = SessionLossCleanup::new(
            CleanupLease::for_writer(lease),
            head.position(),
            original.value().clone(),
            fact.value().clone(),
            native,
            vec![outbox],
        )?;
        self.storage
            .io()
            .session_loss_locked(&disk, cleanup, batch, allocation)
            .await
    }
}
impl SessionFactSink for SessionFactPublisher {
    fn check_current(&self) -> Result<(), CoordinatorError> {
        self.fence.require_connection(&self.ticket)
    }
    fn ended(&self) -> BoxFuture<'static, ()> {
        self.fence.ended(self.ticket.clone())
    }
    fn authentication_requested(
        &self,
        request: Arc<Allocated<LoginProofRequest>>,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        let this = self.clone();
        Box::pin(async move {
            this.fence.check_pending(&this.ticket)?;
            this.ticket.require_authentication(request.value())?;
            let work = this.native_work(false)?;
            let record = JournalRecord::new(
                RecordContext::Run(this.ticket.binding().run().clone()),
                JournalRecordKind::RunStatus,
                Frame::Authentication {
                    attempt: *this.ticket.binding().attempt().as_bytes(),
                    nonce: *this.ticket.binding().nonce().as_bytes(),
                }
                .bytes()?,
            )?;
            let event = CoordinatorEvent::AuthenticationRequested {
                request: Box::new(request.value().clone()),
                startup: this.ticket.startup(),
            };
            this.log
                .append_after_command_resident(&this.original, vec![(record, Some(event))], work)
                .await?;
            this.fence.check_pending(&this.ticket)
        })
    }
    fn established(
        &self,
        candidate: Box<dyn PreparedRunSession>,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        let this = self.clone();
        Box::pin(async move { this.establish(candidate).await })
    }
    fn lost(
        &self,
        observation: Arc<Allocated<SessionObservation>>,
    ) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        // This barrier executes when the physical observation is handed over,
        // before the returned future can wait on disk or publication capacity.
        let observed = self.owner.observe_loss(&self.ticket, observation.value());
        let this = self.clone();
        Box::pin(async move {
            observed?;
            this.publish_loss(observation).await
        })
    }
    fn refused(&self, reason: SessionFailure) -> BoxFuture<'static, Result<(), CoordinatorError>> {
        let this = self.clone();
        Box::pin(async move {
            let allocation = this.resources.fact_metadata(false)?;
            let work = this.native_work(false)?;
            let fact = Allocated::new(
                SessionObservation::new(
                    this.ticket.binding().clone(),
                    SessionOutcome::Refused(reason),
                )?,
                allocation,
            );
            this.owner.abandon(&this.ticket);
            this.fence.check()?;
            this.persist(fact.value(), work).await.map(|_| ())
        })
    }
}
struct PendingCompletion {
    owner: Arc<RunSessionOwner>,
    ticket: SessionTicket,
}
impl Drop for PendingCompletion {
    fn drop(&mut self) {
        self.owner.abandon(&self.ticket);
    }
}
