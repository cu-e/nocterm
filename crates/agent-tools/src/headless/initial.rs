//! Initial physical publication and bounded lifecycle facts from the sole reader.
use super::{
    HeadlessError, HeadlessSessionStatus, HeadlessSessionTask, HeadlessTerminalOptions,
    SessionRegistry,
    pending_open::{InitialPublication, PendingOpen},
};
use futures::future::BoxFuture;
use nocterm_cloud_model::{authority::SessionReceipt, identity::RemoteSessionId, tool::ToolTimer};
use nocterm_session::{Session, SessionError};
use std::{
    fmt,
    sync::{Arc, Mutex, PoisonError},
};

/// An opaque original physical acquisition, independent of descriptive receipts.
#[derive(Clone)]
pub struct SessionGeneration(Arc<()>);
impl PartialEq for SessionGeneration {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for SessionGeneration {}
impl fmt::Debug for SessionGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionGeneration(..)")
    }
}
#[derive(Clone, Debug)]
pub enum SessionLifecycleEvent {
    Connected,
    Lost {
        status: HeadlessSessionStatus,
        error: Option<SessionError>,
    },
}
/// Descriptive original facts. Neither receipt nor generation grants tool access.
#[derive(Clone, Debug)]
pub struct SessionObservation {
    receipt: SessionReceipt,
    generation: SessionGeneration,
    event: SessionLifecycleEvent,
}
impl SessionObservation {
    pub fn receipt(&self) -> &SessionReceipt {
        &self.receipt
    }
    pub fn generation(&self) -> &SessionGeneration {
        &self.generation
    }
    pub fn event(&self) -> &SessionLifecycleEvent {
        &self.event
    }
}
/// One receiver, at most initial Connected and final Lost; no physical/session ref.
pub struct SessionObservations {
    receiver: async_channel::Receiver<SessionObservation>,
}
impl SessionObservations {
    pub fn next(&mut self) -> BoxFuture<'_, Option<SessionObservation>> {
        Box::pin(async move { self.receiver.recv().await.ok() })
    }
}
/// The caller commits Established before consuming this publication capability.
/// Dropping a genuinely pending owner closes only that original physical session.
pub struct InitialSessionCandidate {
    pending: PendingOpen,
    receipt: SessionReceipt,
    generation: SessionGeneration,
}
impl InitialSessionCandidate {
    pub fn prepare(
        registry: Arc<Mutex<SessionRegistry>>,
        handle: RemoteSessionId,
        receipt: SessionReceipt,
        session: Session,
        options: HeadlessTerminalOptions,
        timer: Arc<dyn ToolTimer>,
        residency: Arc<dyn Send + Sync>,
    ) -> Result<(Self, SessionObservations, HeadlessSessionTask), HeadlessError> {
        let (lifecycle, observations) = InitialLifecycle::new(receipt.clone(), residency);
        let generation = lifecycle.generation.clone();
        let task = registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .attach_initial(handle, receipt.clone(), session, options, timer, lifecycle)?;
        let pending = PendingOpen::new(registry, task.terminal().to_owned(), task.original());
        Ok((
            Self {
                pending,
                receipt,
                generation,
            },
            observations,
            task,
        ))
    }
    pub fn receipt(&self) -> &SessionReceipt {
        &self.receipt
    }
    pub fn generation(&self) -> &SessionGeneration {
        &self.generation
    }
    /// Requires the original open view and actual Connected physical generation.
    /// This is the explicit post-durability publication point, not a journal write.
    pub fn publish(self) -> Result<(), HeadlessError> {
        self.pending.publish_initial()
    }
    /// Publishes the actual original physical generation after durability, but
    /// keeps conditional retirement armed until the original run accepts it.
    pub fn publish_owned(self) -> Result<PublishedInitialSession, HeadlessError> {
        let Self {
            pending,
            receipt,
            generation,
        } = self;
        Ok(PublishedInitialSession {
            publication: pending.publish_initial_owned()?,
            receipt,
            generation,
        })
    }
}

/// The coordinator checks source, writer and original pending generation before
/// marking Live and waking that run, then accepts this guard without an await.
/// Until acceptance, cancellation retires only the original physical pointer.
pub struct PublishedInitialSession {
    publication: InitialPublication,
    receipt: SessionReceipt,
    generation: SessionGeneration,
}
impl PublishedInitialSession {
    pub fn receipt(&self) -> &SessionReceipt {
        &self.receipt
    }
    pub fn generation(&self) -> &SessionGeneration {
        &self.generation
    }
    /// The retained physical supervisor continues ownership across run views.
    pub fn accept(self) {
        self.publication.accept();
    }
}

pub(super) struct InitialLifecycle {
    receipt: SessionReceipt,
    generation: SessionGeneration,
    sender: async_channel::Sender<SessionObservation>,
    connected: bool,
    lost: bool,
    _residency: Arc<dyn Send + Sync>,
}
impl InitialLifecycle {
    pub(super) fn matches(&self, observation: &SessionObservation) -> bool {
        self.receipt == observation.receipt && self.generation == observation.generation
    }
    fn new(
        receipt: SessionReceipt,
        residency: Arc<dyn Send + Sync>,
    ) -> (Self, SessionObservations) {
        let (sender, receiver) = async_channel::bounded(2);
        (
            Self {
                receipt,
                generation: SessionGeneration(Arc::new(())),
                sender,
                connected: false,
                lost: false,
                _residency: residency,
            },
            SessionObservations { receiver },
        )
    }
    pub(super) fn connected(&mut self) {
        if !self.connected && !self.lost {
            self.connected = true;
            self.emit(SessionLifecycleEvent::Connected);
        }
    }
    pub(super) fn lost(&mut self, status: HeadlessSessionStatus, error: Option<SessionError>) {
        if !self.lost {
            self.lost = true;
            self.emit(SessionLifecycleEvent::Lost { status, error });
            self.sender.close();
        }
    }
    fn emit(&self, event: SessionLifecycleEvent) {
        // Exactly two possible facts; no terminal output and no unbounded fanout.
        let _ = self.sender.try_send(SessionObservation {
            receipt: self.receipt.clone(),
            generation: self.generation.clone(),
            event,
        });
    }
}
