//! Raw connection operations with explicit, prepaid correlation ownership.
use super::cancellation::Cancellation;
use super::{Budget, Buffer, Envelope, ManagedError, MessageId, MessageKind, Shared, Weak};
use super::{
    map::Map,
    queue::Queue,
    reply::{Failure, ReplySlot, Request},
    respond::{Group, Reply},
};
use crate::jsonrpc::routing::{CancellationState, PendingRegistry};
use std::{
    sync::Mutex,
    task::{Context, Poll, Waker},
};

struct Registration<B: Budget> {
    generation: u64,
    state: CancellationState<Cancellation<B>>,
}
pub(super) enum Queued<B: Budget> {
    Request(Envelope<B>, Shared<ReplySlot<B>, B>),
    Notification(Envelope<B>),
}
pub(super) struct Core<B: Budget> {
    pending: PendingRegistry<Map<MessageId<B>, Shared<ReplySlot<B>, B>, B>>,
    cancellations: Map<MessageId<B>, Registration<B>, B>,
    outgoing: Queue<Queued<B>, B>,
    groups: Buffer<Group<B>, B>,
    driver_waker: Option<Waker>,
    budget: B,
    next_generation: u64,
}
/// Weak original connection handle. Escaped handles retain only its paid header,
/// never pending response senders or the native transport lifetime.
pub struct Connection<B: Budget> {
    core: Weak<Mutex<Core<B>>, B>,
}
impl<B: Budget> Clone for Connection<B> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
        }
    }
}
/// Driver-owned raw protocol routing. The adapter supplies the actual transport
/// and native codec futures; the SDK creates no hidden executor tasks/channels.
pub struct Driver<B: Budget> {
    core: Shared<Mutex<Core<B>>, B>,
}
/// An owned output operation for the controlled native encoder.
pub enum Outgoing<B: Budget> {
    /// Original complete native-encoded message, retaining its physical source.
    Message(Envelope<B>),
    /// Encode original `$/cancel_request` for an already published request.
    Cancel(MessageId<B>),
    /// A single response or original ordered batch of responses.
    Responses {
        /// Whether native encoding uses the original batch array framing.
        batch: bool,
        /// Actual guarded response owners in original batch slot order.
        replies: Buffer<Reply<B>, B>,
    },
}
impl<B: Budget> Driver<B> {
    /// Create one original connection and explicit weak operation handle.
    pub fn new(budget: B) -> Result<(Self, Connection<B>), ManagedError<B::Error>> {
        let core = Shared::new(
            Mutex::new(Core {
                pending: PendingRegistry::new(Map::new(budget.clone())),
                cancellations: Map::new(budget.clone()),
                outgoing: Queue::new(budget.clone()),
                groups: Buffer::new(budget.clone()),
                driver_waker: None,
                next_generation: 0,
                budget: budget.clone(),
            }),
            &budget,
        )?;
        let connection = Connection {
            core: core.downgrade(),
        };
        Ok((Self { core }, connection))
    }
    pub(super) fn core(&self) -> &Shared<Mutex<Core<B>>, B> {
        &self.core
    }
    /// Admit actual source-order frame processing without typed JSON decoding.
    pub fn receive(
        &mut self,
        frame: super::Frame<B>,
    ) -> Result<super::Reception<B>, ManagedError<B::Error>> {
        super::Reception::new(self, frame)
    }
    /// Poll actual queued output. Cancellation before publication discards the
    /// original queued request; cancellation afterward emits its original ID.
    pub fn poll_outgoing(&mut self, cx: &mut Context<'_>) -> Poll<Option<Outgoing<B>>> {
        loop {
            let waiter = cx.waker().clone();
            let (queued, previous) = {
                let mut core = self
                    .core
                    .get()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let provisional = matches!(core.outgoing.front(),Some(Queued::Request(_,slot)) if !slot.get().is_publishable() && !slot.get().is_cancelled());
                let queued = if provisional { None } else { core.outgoing.pop() };
                let previous = if queued.is_none() {
                    core.driver_waker.replace(waiter)
                } else {
                    None
                };
                (queued, previous)
            };
            drop(previous);
            match queued {
                Some(Queued::Notification(envelope)) => {
                    return Poll::Ready(Some(Outgoing::Message(envelope)));
                }
                Some(Queued::Request(envelope, slot)) => {
                    if slot.get().is_cancelled() {
                        let removed = self
                            .core
                            .get()
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .pending
                            .remove(slot.get().id());
                        drop(removed);
                        drop(envelope);
                        continue;
                    }
                    slot.get().mark_published();
                    return Poll::Ready(Some(Outgoing::Message(envelope)));
                }
                None => {}
            }
            if let Some(id) = self.take_cancelled() {
                return Poll::Ready(Some(Outgoing::Cancel(id)));
            }
            if let Some(output) = self.take_responses() {
                return Poll::Ready(Some(output));
            }
            let closed = self
                .core
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pending
                .is_closed();
            return if closed {
                Poll::Ready(None)
            } else {
                Poll::Pending
            };
        }
    }
    fn take_cancelled(&self) -> Option<MessageId<B>> {
        let removed = {
            let mut core = self
                .core
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let id = core
                .pending
                .values()
                .find(|slot| slot.get().is_cancelled() && slot.get().is_published())
                .map(|slot| slot.get().id().clone());
            id.and_then(|id| core.pending.remove(&id).map(|slot| (id, slot)))
        };
        removed.map(|(id, slot)| {
            drop(slot);
            id
        })
    }
    fn take_responses(&self) -> Option<Outgoing<B>> {
        loop {
            let mut core = self
                .core
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let ready = core
                .groups
                .as_slice()
                .iter()
                .enumerate()
                .find_map(|(index, group)| {
                    group
                        .take_ready(core.budget.clone())
                        .map(|output| (index, output))
                });
            let (index, (batch, replies)) = ready?;
            let group = core.groups.remove(index);
            drop(core);
            drop(group);
            if !replies.is_empty() {
                return Some(Outgoing::Responses { batch, replies });
            }
        }
    }
    /// Mark the original incoming request cancellation, without replacing its
    /// responder or affecting a stale registration that reused the same ID.
    pub fn cancel_incoming(&self, id: &MessageId<B>) -> bool {
        let (found, marker) = {
            let mut core = self
                .core
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            match core.cancellations.get_mut(id) {
                Some(registration) => (true, registration.state.cancel()),
                None => (false, None),
            }
        };
        if let Some(marker) = marker {
            marker.cancel();
        }
        found
    }

    /// Atomically close subscriptions, then fail original request waiters
    /// outside the registry lock. Actual source buffers remain with their owners.
    pub fn close(&mut self) {
        let pending = {
            let mut core = self
                .core
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            core.pending.close()
        };
        for slot in pending {
            slot.get().deliver(Err(Failure::Closed));
        }
        wake_driver(&self.core);
    }
    pub(super) fn register_group(&self, group: Group<B>) -> Result<(), ManagedError<B::Error>> {
        let mut core = self
            .core
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        core.groups.push(group).map_err(|(error, _)| error)
    }
    pub(super) fn route_response(&self, envelope: Envelope<B>) -> Option<Shared<ReplySlot<B>, B>> {
        let id = envelope.id()?;
        let slot = {
            let mut core = self.core.get().lock().unwrap_or_else(|error|error.into_inner());
            if core.pending.get(id).is_some_and(|slot|!slot.get().is_publishable()) { return None; }
            core.pending.remove(id)?
        };
        if slot.get().is_cancelled() {
            return None;
        }
        if slot.get().deliver(Ok(envelope)) {
            Some(slot)
        } else {
            None
        }
    }
}
impl<B: Budget> Drop for Driver<B> {
    fn drop(&mut self) {
        self.close();
    }
}
impl<B: Budget> Connection<B> {
    /// Create the original SDK UUID-v4 wire ID with prepaid UTF-8 storage.
    /// Entropy and spelling match the original typed request path; the native
    /// allocation authority must approve storage before its source is copied.
    pub fn next_id(&self) -> Result<MessageId<B>, ManagedError<B::Error>> {
        let connection = self.core.upgrade().ok_or(ManagedError::Closed)?;
        let budget = {
            let core = connection
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if core.pending.is_closed() {
                return Err(ManagedError::Closed);
            }
            core.budget.clone()
        };
        let uuid = uuid::Uuid::new_v4();
        let mut bytes = uuid::Uuid::encode_buffer();
        let text = uuid.hyphenated().encode_lower(&mut bytes);
        Ok(MessageId::String(super::Source::copy(text, budget)?.full()))
    }
    /// Admit a complete native-encoded request before its queue publication.
    /// Refusal returns the same original envelope, without a pending entry.
    pub fn request(
        &self,
        envelope: Envelope<B>,
    ) -> Result<Request<B>, (ManagedError<B::Error>, Envelope<B>)> {
        self.admit_request(envelope,true)
    }
    /// Admit the actual pending and FIFO positions now; first polling the same
    /// Request activates publication without allocating. An unpolled drop emits
    /// nothing and preserves original cancellation cleanup.
    pub fn prepare_request(&self,envelope:Envelope<B>)->Result<Request<B>,(ManagedError<B::Error>,Envelope<B>)> {
        self.admit_request(envelope,false)
    }
    fn admit_request(&self,envelope:Envelope<B>,publishable:bool)->Result<Request<B>,(ManagedError<B::Error>,Envelope<B>)> {
        if envelope.kind() != MessageKind::Request {
            return Err((ManagedError::InvalidEnvelope, envelope));
        }
        let Some(connection) = self.core.upgrade() else {
            return Err((ManagedError::Closed, envelope));
        };
        let admitted = {
            let mut core = connection
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            core.admit(envelope,publishable)
        };
        let (slot, replaced) = admitted?;
        if let Some(replaced) = replaced {
            replaced.get().deliver(Err(Failure::Reused));
        }
        wake_driver(&connection);
        Ok(Request::new(slot, self.core.clone()))
    }

    /// Admit an actual native-encoded notification, preserving source ownership.
    pub fn notify(
        &self,
        envelope: Envelope<B>,
    ) -> Result<(), (ManagedError<B::Error>, Envelope<B>)> {
        if envelope.kind() != MessageKind::Notification {
            return Err((ManagedError::InvalidEnvelope, envelope));
        }
        let Some(connection) = self.core.upgrade() else {
            return Err((ManagedError::Closed, envelope));
        };
        let admitted = {
            let mut core = connection
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if core.pending.is_closed() {
                Err((ManagedError::Closed, envelope))
            } else if !envelope.belongs_to(&core.budget) {
                Err((ManagedError::ForeignResponse, envelope))
            } else if let Err(error) = core.outgoing.reserve() {
                Err((error, envelope))
            } else {
                core.outgoing.push_reserved(Queued::Notification(envelope));
                Ok(())
            }
        };
        admitted?;
        wake_driver(&connection);
        Ok(())
    }
}
impl<B: Budget> Core<B> {
    pub(super) fn is_closed(&self) -> bool {
        self.pending.is_closed()
    }
    pub(super) fn budget(&self) -> B {
        self.budget.clone()
    }
    pub(super) fn register_cancellation(
        &mut self,
        id: MessageId<B>,
    ) -> Result<u64, ManagedError<B::Error>> {
        self.cancellations.reserve()?;
        let generation = self.next_generation;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(ManagedError::CapacityOverflow)?;
        self.cancellations.insert_reserved(
            id,
            Registration {
                generation,
                state: CancellationState::Armed,
            },
        );
        Ok(generation)
    }
    pub(super) fn marker(
        &mut self,
        id: &MessageId<B>,
        generation: u64,
    ) -> Result<Cancellation<B>, ManagedError<B::Error>> {
        let budget = self.budget.clone();
        if let Some(registration) = self.cancellations.get_mut(id) {
            if registration.generation == generation {
                return registration.state.marker(|cancelled| {
                    let marker = Cancellation::new(&budget)?;
                    if cancelled {
                        marker.cancel();
                    }
                    Ok(marker)
                });
            }
        }
        Cancellation::new(&budget)
    }
    pub(super) fn release_marker(&mut self, id: &MessageId<B>, generation: u64) {
        if self
            .cancellations
            .get(id)
            .is_some_and(|current| current.generation == generation)
        {
            self.cancellations.remove(id);
        }
    }

    fn admit(
        &mut self,
        envelope: Envelope<B>,
        publishable: bool,
    ) -> Result<
        (Shared<ReplySlot<B>, B>, Option<Shared<ReplySlot<B>, B>>),
        (ManagedError<B::Error>, Envelope<B>),
    > {
        let prepared = (|| {
            if self.pending.is_closed() {
                return Err(ManagedError::Closed);
            }
            if !envelope.belongs_to(&self.budget) {
                return Err(ManagedError::ForeignResponse);
            }
            let id = envelope.id().ok_or(ManagedError::InvalidEnvelope)?;
            if !publishable && self.pending.contains(id) { return Err(ManagedError::ForeignResponse); }
            self.outgoing.reserve()?;
            if !self.pending.prepare()? {
                return Err(ManagedError::Closed);
            }
            let id = envelope.id().ok_or(ManagedError::InvalidEnvelope)?.clone();
            let slot = Shared::new(
                ReplySlot::new(id.clone(), envelope.original().clone(),publishable),
                &self.budget,
            )?;
            Ok((id, slot))
        })();
        let (id, slot) = match prepared {
            Ok(value) => value,
            Err(error) => return Err((error, envelope)),
        };
        let replaced = self
            .pending
            .subscribe_reserved(id, slot.clone())
            .unwrap_or_else(|_| unreachable!("original registry lock"));
        self.outgoing
            .push_reserved(Queued::Request(envelope, slot.clone()));
        Ok((slot, replaced))
    }
}
pub(super) fn wake_driver<B: Budget>(connection: &Shared<Mutex<Core<B>>, B>) {
    let wake = connection
        .get()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .driver_waker
        .take();
    if let Some(wake) = wake {
        wake.wake();
    }
}
