//! Original-order frame dispatch; raw response futures retain real acknowledgement.
use super::{
    Attempt, Budget, Entry, Envelope, Frame, ManagedError, MessageId, MessageKind, Responder, Weak,
};
use super::{
    buffer::Values,
    connection::{Core, Driver},
    reply::ReplySlot,
    respond::{Fault, Group, Reply},
};
use std::{
    sync::Mutex,
    task::{Context, Poll},
};

enum Entries<B: Budget> {
    Single(Option<Entry<B>>),
    Batch(Values<Entry<B>, B>),
}
impl<B: Budget> Entries<B> {
    fn next(&mut self) -> Option<Entry<B>> {
        match self {
            Self::Single(entry) => entry.take(),
            Self::Batch(entries) => entries.next(),
        }
    }
}
/// One raw notification or original request and its real handler-attempt guard.
/// The adapter may move its original responder into permission work; retain this
/// dispatch while the actual handler attempt runs.
pub struct Dispatch<B: Budget> {
    envelope: Envelope<B>,
    responder: Option<Responder<B>>,
    _attempt: Option<Attempt<B>>,
}
impl<B: Budget> Dispatch<B> {
    /// Borrow actual raw method data for controlled native decoding.
    pub fn envelope(&self) -> &Envelope<B> {
        &self.envelope
    }
    /// Move the original responder into actual native/permission work.
    pub fn take_responder(&mut self) -> Option<Responder<B>> {
        self.responder.take()
    }
    /// Apply original unhandled behavior: ignore notifications, reject calls.
    pub fn unhandled(mut self) {
        if let Some(responder) = self.responder.take() {
            responder.unhandled();
        }
    }
}
/// One actual frame's source-order processing and original batch completion.
/// Responses may suspend dispatch until their original ordered consumer releases
/// its response guard. No typed response/value is copied into this owner.
pub struct Reception<B: Budget> {
    entries: Entries<B>,
    group: Option<Group<B>>,
    response_index: usize,
    waiting: Option<super::Shared<ReplySlot<B>, B>>,
    connection: Weak<Mutex<Core<B>>, B>,
    finished: bool,
}
fn response_required<B: Budget>(entry: &Entry<B>) -> bool {
    match entry {
        Entry::Message(envelope) => envelope.kind() == MessageKind::Request,
        Entry::Malformed(malformed) => !malformed.response_only(),
    }
}
impl<B: Budget> Reception<B> {
    /// Admit a complete actual frame. Source-order entries and original malformed
    /// positions remain owned until dispatch, failure or cancellation drops them.
    pub(super) fn new(driver: &Driver<B>, frame: Frame<B>) -> Result<Self, ManagedError<B::Error>> {
        {
            let core = driver
                .core()
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if core.is_closed() {
                return Err(ManagedError::Closed);
            }
            if !frame.belongs_to(&core.budget()) {
                return Err(ManagedError::ForeignResponse);
            }
        }
        let (count, batch) = match &frame {
            Frame::Single(envelope) => {
                (usize::from(envelope.kind() == MessageKind::Request), false)
            }
            Frame::Batch(entries) => (
                entries
                    .as_slice()
                    .iter()
                    .filter(|entry| response_required(entry))
                    .count(),
                true,
            ),
            Frame::Malformed(malformed) => (usize::from(!malformed.response_only()), false),
        };
        let budget = driver
            .core()
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .budget();
        let group = if count == 0 {
            None
        } else {
            let group = Group::new(budget, count, batch)?;
            driver.register_group(group.clone())?;
            Some(group)
        };
        let entries = match frame {
            Frame::Single(envelope) => Entries::Single(Some(Entry::Message(envelope))),
            Frame::Batch(entries) => Entries::Batch(entries.into_values()),
            Frame::Malformed(malformed) => Entries::Single(Some(Entry::Malformed(malformed))),
        };
        Ok(Reception {
            entries,
            group,
            response_index: 0,
            waiting: None,
            connection: driver.core().downgrade(),
            finished: false,
        })
    }
}
impl<B: Budget> Reception<B> {
    /// Poll the next original request/notification; replies route to their own
    /// future. A foreign driver is rejected rather than renewing authority.
    pub fn poll_dispatch(
        &mut self,
        driver: &mut Driver<B>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Option<Dispatch<B>>, ManagedError<B::Error>>> {
        let Some(original) = self.connection.upgrade() else {
            return Poll::Ready(Err(ManagedError::Closed));
        };
        if !original.same_allocation(driver.core()) {
            return Poll::Ready(Err(ManagedError::ForeignResponse));
        }
        if let Some(waiting) = &self.waiting {
            if waiting.get().poll_ack(cx).is_pending() {
                return Poll::Pending;
            }
            self.waiting = None;
        }
        loop {
            let Some(entry) = self.entries.next() else {
                if !self.finished {
                    if let Some(group) = &self.group {
                        group.finish_dispatch();
                    }
                    super::connection::wake_driver(driver.core());
                    self.finished = true;
                }
                return Poll::Ready(Ok(None));
            };
            let envelope = match entry {
                Entry::Message(envelope) => envelope,
                Entry::Malformed(malformed) => {
                    if !malformed.response_only() {
                        let fault = if malformed.parse_error() {
                            Fault::Parse
                        } else {
                            Fault::InvalidRequest
                        };
                        if let Some(group) = &self.group {
                            group.complete(
                                self.response_index,
                                Reply::Fault {
                                    id: MessageId::Null,
                                    fault,
                                    method: None,
                                    original: Some(malformed.original().clone()),
                                },
                            );
                            self.response_index += 1;
                        }
                    }
                    continue;
                }
            };
            match envelope.kind() {
                MessageKind::Result | MessageKind::Error => {
                    if let Some(waiting) = driver.route_response(envelope) {
                        self.waiting = Some(waiting);
                        if let Some(waiting) = &self.waiting {
                            if waiting.get().poll_ack(cx).is_pending() {
                                return Poll::Pending;
                            }
                        }
                        self.waiting = None;
                    }
                }
                MessageKind::Notification => {
                    return Poll::Ready(Ok(Some(Dispatch {
                        envelope,
                        responder: None,
                        _attempt: None,
                    })));
                }
                MessageKind::Request => {
                    return Poll::Ready(self.request(driver, envelope).map(Some));
                }
            }
        }
    }
    fn request(
        &mut self,
        driver: &Driver<B>,
        envelope: Envelope<B>,
    ) -> Result<Dispatch<B>, ManagedError<B::Error>> {
        let id = envelope.id().ok_or(ManagedError::InvalidEnvelope)?.clone();
        let method = envelope
            .method_span()
            .ok_or(ManagedError::InvalidEnvelope)?;
        let (generation, budget) = {
            let mut core = driver
                .core()
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let generation = core.register_cancellation(id.clone())?;
            (generation, core.budget())
        };
        let group = self
            .group
            .as_ref()
            .ok_or(ManagedError::InvalidEnvelope)?
            .clone();
        let responder = Responder::new(
            group,
            self.response_index,
            id,
            method,
            generation,
            driver.core().downgrade(),
            budget,
        );
        self.response_index += 1;
        let attempt = responder.begin_attempt();
        Ok(Dispatch {
            envelope,
            responder: Some(responder),
            _attempt: Some(attempt),
        })
    }
}
