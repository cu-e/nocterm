//! One actual bounded FIFO with paid slots and cancellation-safe sender waiters.
use super::budget::SourceBudget;
use agent_client_protocol::managed::{Buffer, ManagedError, Shared};
use nocterm_ai::model::json::deferred::JsonDecodeError;
use std::{
    future::Future,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll, Waker},
};
type Error = ManagedError<JsonDecodeError>;
#[derive(Clone, Copy)]
struct Ticket {
    index: usize,
    generation: u64,
}
struct Waiter {
    generation: u64,
    previous: Option<usize>,
    next: Option<usize>,
    waker: Option<Waker>,
    active: bool,
}
struct Waiters {
    slots: Buffer<Waiter, SourceBudget>,
    free: Option<usize>,
    first: Option<usize>,
    last: Option<usize>,
}
impl Waiters {
    fn new(budget: SourceBudget) -> Self {
        Self {
            slots: Buffer::new(budget),
            free: None,
            first: None,
            last: None,
        }
    }
    fn register(
        &mut self,
        ticket: Option<Ticket>,
        waker: Waker,
    ) -> Result<(Ticket, Option<Waker>), (Error, Waker)> {
        if let Some(ticket) = ticket {
            let entry = &mut self.slots.as_mut_slice()[ticket.index];
            if entry.active && entry.generation == ticket.generation {
                return Ok((ticket, entry.waker.replace(waker)));
            }
        }
        let index = if let Some(index) = self.free {
            index
        } else {
            let index = self.slots.len();
            if let Err((error, _)) = self.slots.push(Waiter {
                generation: 0,
                previous: None,
                next: None,
                waker: None,
                active: false,
            }) {
                return Err((error, waker));
            }
            index
        };
        let Some(generation) = self.slots.as_slice()[index].generation.checked_add(1) else {
            return Err((ManagedError::CapacityOverflow, waker));
        };
        if self.free == Some(index) {
            self.free = self.slots.as_slice()[index].next;
        }
        let entry = &mut self.slots.as_mut_slice()[index];
        *entry = Waiter {
            generation,
            previous: self.last,
            next: None,
            waker: Some(waker),
            active: true,
        };
        if let Some(last) = self.last {
            self.slots.as_mut_slice()[last].next = Some(index);
        } else {
            self.first = Some(index);
        }
        self.last = Some(index);
        Ok((Ticket { index, generation }, None))
    }
    fn remove(&mut self, ticket: Ticket) -> Option<Waker> {
        let entry = &self.slots.as_slice()[ticket.index];
        if !entry.active || entry.generation != ticket.generation {
            return None;
        }
        let (previous, next) = (entry.previous, entry.next);
        if let Some(index) = previous {
            self.slots.as_mut_slice()[index].next = next;
        } else {
            self.first = next;
        }
        if let Some(index) = next {
            self.slots.as_mut_slice()[index].previous = previous;
        } else {
            self.last = previous;
        }
        let entry = &mut self.slots.as_mut_slice()[ticket.index];
        entry.active = false;
        entry.next = self.free;
        self.free = Some(ticket.index);
        entry.waker.take()
    }
    fn wake_first(&mut self) -> Option<Waker> {
        self.first
            .and_then(|index| self.slots.as_mut_slice()[index].waker.take())
    }
    fn wake_all(mut self) {
        for entry in self.slots.as_mut_slice() {
            if let Some(waker) = entry.waker.take() {
                waker.wake();
            }
        }
    }
}
struct State<T> {
    values: Buffer<Option<T>, SourceBudget>,
    head: usize,
    len: usize,
    senders: usize,
    closed: bool,
    receiving: bool,
    receiver: Option<Waker>,
    waiters: Waiters,
    budget: SourceBudget,
}
impl<T> State<T> {
    fn dequeue(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let result = self.values.as_mut_slice()[self.head].take();
        self.head = (self.head + 1) % self.values.len();
        self.len -= 1;
        result
    }
    fn enqueue(&mut self, value: T) {
        let index = (self.head + self.len) % self.values.len();
        self.values.as_mut_slice()[index] = Some(value);
        self.len += 1;
    }
    fn close(&mut self) -> (Option<Waker>, Waiters) {
        self.closed = true;
        (
            self.receiver.take(),
            std::mem::replace(&mut self.waiters, Waiters::new(self.budget.clone())),
        )
    }
}
pub(super) struct Sender<T> {
    state: Shared<Mutex<State<T>>, SourceBudget>,
}
pub(super) struct Receiver<T> {
    state: Shared<Mutex<State<T>>, SourceBudget>,
}
pub(super) fn bounded<T>(
    budget: SourceBudget,
    capacity: usize,
) -> Result<(Sender<T>, Receiver<T>), Error> {
    if capacity == 0 {
        return Err(ManagedError::InvalidEnvelope);
    }
    let mut values = Buffer::new(budget.clone());
    values.reserve(capacity)?;
    for _ in 0..capacity {
        values.push(None).map_err(|(error, _)| error)?;
    }
    let state = Shared::new(
        Mutex::new(State {
            values,
            head: 0,
            len: 0,
            senders: 1,
            closed: false,
            receiving: true,
            receiver: None,
            waiters: Waiters::new(budget.clone()),
            budget: budget.clone(),
        }),
        &budget,
    )?;
    Ok((
        Sender {
            state: state.clone(),
        },
        Receiver { state },
    ))
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.state
            .get()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .senders += 1;
        Self {
            state: self.state.clone(),
        }
    }
}
#[derive(Debug)]
pub(super) enum TrySendError<T> {
    Full(T),
    Closed(T),
}
impl<T> Sender<T> {
    pub(super) fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        let receiver = {
            let mut state = self.state.get().lock().unwrap_or_else(|e| e.into_inner());
            if state.closed || !state.receiving {
                return Err(TrySendError::Closed(value));
            }
            if state.len == state.values.len() {
                return Err(TrySendError::Full(value));
            }
            state.enqueue(value);
            state.receiver.take()
        };
        if let Some(waker) = receiver {
            waker.wake();
        }
        Ok(())
    }

    /// The original final-exit event evicts exactly one oldest position only
    /// when the existing event queue is full. Displaced payload drops outside
    /// the shared mutex and never acknowledges a discarded barrier.
    pub(super) fn publish_final(&self, value: T) -> Result<Option<T>, T> {
        let (displaced, receiver) = {
            let mut state = self.state.get().lock().unwrap_or_else(|e| e.into_inner());
            if state.closed || !state.receiving {
                return Err(value);
            }
            let displaced = if state.len == state.values.len() {
                state.dequeue()
            } else {
                None
            };
            state.enqueue(value);
            (displaced, state.receiver.take())
        };
        if let Some(waker) = receiver {
            waker.wake();
        }
        Ok(displaced)
    }

    pub(super) fn send(&self, value: T) -> Send<T> {
        Send {
            state: self.state.clone(),
            value: Some(value),
            ticket: None,
            done: false,
        }
    }
    pub(super) fn close(&self) {
        let (receiver, waiters) = self
            .state
            .get()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .close();
        if let Some(waker) = receiver {
            waker.wake();
        }
        waiters.wake_all();
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let wake = {
            let mut state = self.state.get().lock().unwrap_or_else(|e| e.into_inner());
            state.senders -= 1;
            if state.senders == 0 {
                Some(state.close())
            } else {
                None
            }
        };
        if let Some((receiver, waiters)) = wake {
            if let Some(waker) = receiver {
                waker.wake();
            }
            waiters.wake_all();
        }
    }
}
pub(super) struct Send<T> {
    state: Shared<Mutex<State<T>>, SourceBudget>,
    value: Option<T>,
    ticket: Option<Ticket>,
    done: bool,
}
#[derive(Debug)]
pub(super) enum SendError<T> {
    Closed(T),
    Allocation(Error, T),
}
impl<T> Unpin for Send<T> {}
impl<T> Future for Send<T> {
    type Output = Result<(), SendError<T>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let Some(value) = this.value.take() else {
            return Poll::Pending;
        };
        let waker = cx.waker().clone();
        let mut state = this.state.get().lock().unwrap_or_else(|e| e.into_inner());
        if state.closed || !state.receiving {
            this.ticket = None;
            this.done = true;
            drop(state);
            drop(waker);
            return Poll::Ready(Err(SendError::Closed(value)));
        }
        if state.len < state.values.len() {
            let old = this
                .ticket
                .take()
                .and_then(|ticket| state.waiters.remove(ticket));
            state.enqueue(value);
            this.done = true;
            let receiver = state.receiver.take();
            drop(state);
            drop(old);
            drop(waker);
            if let Some(waker) = receiver {
                waker.wake();
            }
            return Poll::Ready(Ok(()));
        }
        match state.waiters.register(this.ticket, waker) {
            Ok((ticket, old)) => {
                this.ticket = Some(ticket);
                this.value = Some(value);
                drop(state);
                drop(old);
                Poll::Pending
            }
            Err((error, waker)) => {
                drop(state);
                drop(waker);
                this.done = true;
                Poll::Ready(Err(SendError::Allocation(error, value)))
            }
        }
    }
}
impl<T> Drop for Send<T> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let (old, wake) = {
            let mut state = self.state.get().lock().unwrap_or_else(|e| e.into_inner());
            let old = if state.closed {
                None
            } else {
                self.ticket
                    .take()
                    .and_then(|ticket| state.waiters.remove(ticket))
            };
            let wake = if state.len < state.values.len() {
                state.waiters.wake_first()
            } else {
                None
            };
            (old, wake)
        };
        drop(old);
        if let Some(waker) = wake {
            waker.wake();
        }
    }
}
impl<T> Receiver<T> {
    #[cfg(test)]
    pub(super) fn try_recv(&mut self) -> Option<T> {
        let (value, waker) = {
            let mut state = self.state.get().lock().unwrap_or_else(|e| e.into_inner());
            let value = state.dequeue();
            let waker = if value.is_some() {
                state.waiters.wake_first()
            } else {
                None
            };
            (value, waker)
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        value
    }
    pub(super) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let waker = cx.waker().clone();
        let mut state = self.state.get().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = state.dequeue() {
            let sender = state.waiters.wake_first();
            drop(state);
            drop(waker);
            if let Some(sender) = sender {
                sender.wake();
            }
            return Poll::Ready(Some(value));
        }
        if state.closed {
            drop(state);
            drop(waker);
            return Poll::Ready(None);
        }
        let old = state.receiver.replace(waker);
        drop(state);
        drop(old);
        Poll::Pending
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let (mut values, receiver, waiters) = {
            let mut state = self.state.get().lock().unwrap_or_else(|e| e.into_inner());
            state.receiving = false;
            let (receiver, waiters) = state.close();
            let budget = state.budget.clone();
            let values = std::mem::replace(&mut state.values, Buffer::new(budget));
            state.len = 0;
            (values, receiver, waiters)
        };
        values.clear();
        drop(values);
        drop(receiver);
        waiters.wake_all();
    }
}
