//! Original reply futures and response acknowledgement, with no boxed channel.
use super::connection::Core;
use super::{Budget, Envelope, ManagedError, MessageId, Shared, Span, Weak};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};

#[derive(Clone, Copy)]
pub(super) enum Failure {
    Closed,
    Reused,
}
pub(super) struct ReplyState<B: Budget> {
    response: Option<Result<Envelope<B>, Failure>>,
    waiter: Option<Waker>,
    ack_waiter: Option<Waker>,
    ordered: bool,
    ack_pending: bool,
}
pub(super) struct ReplySlot<B: Budget> {
    id: MessageId<B>,
    state: Mutex<ReplyState<B>>,
    cancelled: AtomicBool,
    published: AtomicBool,
    publishable: AtomicBool,
    _original: Span<B>,
}
impl<B: Budget> ReplySlot<B> {
    pub(super) fn new(id: MessageId<B>, original: Span<B>, publishable: bool) -> Self {
        Self {
            id,
            state: Mutex::new(ReplyState {
                response: None,
                waiter: None,
                ack_waiter: None,
                ordered: false,
                ack_pending: false,
            }),
            cancelled: AtomicBool::new(false),
            published: AtomicBool::new(false),
            publishable: AtomicBool::new(publishable),
            _original: original,
        }
    }
    pub(super) fn id(&self) -> &MessageId<B> {
        &self.id
    }
    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub(super) fn is_published(&self) -> bool {
        self.published.load(Ordering::Acquire)
    }
    pub(super) fn is_publishable(&self) -> bool { self.publishable.load(Ordering::Acquire) }
    fn activate(&self) -> bool { !self.publishable.swap(true,Ordering::AcqRel) }
    pub(super) fn mark_published(&self) {
        self.published.store(true, Ordering::Release);
    }
    pub(super) fn deliver(&self, response: Result<Envelope<B>, Failure>) -> bool {
        let (wake, ordered) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.response.is_some() || self.cancelled.load(Ordering::Acquire) {
                return false;
            }
            let ordered = state.ordered && response.is_ok();
            state.ack_pending = ordered;
            state.response = Some(response);
            (state.waiter.take(), ordered)
        };
        if let Some(wake) = wake {
            wake.wake();
        }
        ordered
    }
    pub(super) fn poll_ack(&self, cx: &mut Context<'_>) -> Poll<()> {
        let waiter = cx.waker().clone();
        let previous = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if !state.ack_pending {
                return Poll::Ready(());
            }
            state.ack_waiter.replace(waiter)
        };
        drop(previous);
        Poll::Pending
    }
    fn acknowledge(&self) {
        let wake = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.ack_pending = false;
            state.ack_waiter.take()
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}
/// One original outgoing operation. Its source survives unpolled cancellation.
pub struct Request<B: Budget> {
    slot: Shared<ReplySlot<B>, B>,
    connection: Weak<Mutex<Core<B>>, B>,
    finished: bool,
}
impl<B: Budget> Request<B> {
    pub(super) fn new(slot: Shared<ReplySlot<B>, B>, connection: Weak<Mutex<Core<B>>, B>) -> Self {
        Self {
            slot,
            connection,
            finished: false,
        }
    }
    /// Hold source-order dispatch until the actual response guard is dropped.
    /// Select this before the original response is routed.
    pub fn ordered(self) -> Self {
        self.slot
            .get()
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .ordered = true;
        self
    }
}
impl<B: Budget> Unpin for Request<B> {}
impl<B: Budget> Future for Request<B> {
    type Output = Result<Response<B>, ManagedError<B::Error>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.slot.get().activate() {
            if let Some(connection) = this.connection.upgrade() { super::connection::wake_driver(&connection); }
        }
        let waiter = cx.waker().clone();
        let (response, previous) = {
            let mut state = this
                .slot
                .get()
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let response = state.response.take();
            let previous = if response.is_none() {
                state.waiter.replace(waiter)
            } else {
                None
            };
            (response, previous)
        };
        drop(previous);
        let Some(response) = response else {
            return Poll::Pending;
        };
        this.finished = true;
        Poll::Ready(match response {
            Ok(envelope) => Ok(Response {
                envelope: Some(envelope),
                ack: Some(this.slot.clone()),
            }),
            Err(Failure::Closed) => Err(ManagedError::Closed),
            Err(Failure::Reused) => Err(ManagedError::ForeignResponse),
        })
    }
}
impl<B: Budget> Drop for Request<B> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.slot.get().cancelled.store(true, Ordering::Release);
        let undelivered = {
            self.slot
                .get()
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .response
                .take()
        };
        drop(undelivered);
        self.slot.get().acknowledge();
        if let Some(connection) = self.connection.upgrade() {
            super::connection::wake_driver(&connection);
        }
    }
}
/// Actual guarded response. Decode by borrowing; dropping it releases ordering.
/// Remote error bodies remain original raw source, not an SDK error DTO.
pub struct Response<B: Budget> {
    envelope: Option<Envelope<B>>,
    ack: Option<Shared<ReplySlot<B>, B>>,
}
impl<B: Budget> Response<B> {
    /// Borrow original response payload and its authentic source.
    pub fn envelope(&self) -> &Envelope<B> {
        self.envelope.as_ref().expect("live response")
    }
}
impl<B: Budget> Drop for Response<B> {
    fn drop(&mut self) {
        drop(self.envelope.take());
        if let Some(ack) = self.ack.take() {
            ack.get().acknowledge();
        }
    }
}
