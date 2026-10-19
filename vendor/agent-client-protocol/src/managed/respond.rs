//! Prepaid incoming response slots share the SDK's batch lifecycle transitions.
use super::cancellation::Cancellation;
use super::connection::{Core, wake_driver};
use super::{Budget, Buffer, Envelope, ManagedError, MessageId, MessageKind, Shared, Span, Weak};
use crate::jsonrpc::routing::BatchLifecycle;
use std::sync::Mutex;

/// A protocol-owned error whose strings/JSON are produced by the native codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Original lexical JSON parse error.
    Parse,
    /// Original invalid JSON-RPC envelope or batch entry.
    InvalidRequest,
    /// Original unhandled-method response; data contains the method name.
    MethodNotFound,
    /// A batch handler dropped its original responder; data describes the method.
    Abandoned,
}
/// One actual output body, including allocation-free protocol error directives.
pub enum Reply<B: Budget> {
    /// A complete native-encoded guarded response envelope.
    Message(Envelope<B>),
    /// Encode the SDK's original fault using the supplied guarded routing facts.
    Fault {
        /// Original request ID or Null for malformed input.
        id: MessageId<B>,
        /// Original error classification.
        fault: Fault,
        /// Actual original decoded method, where the error includes it.
        method: Option<Span<B>>,
        /// Original malformed line, when lexical fault data includes it.
        original: Option<Span<B>>,
    },
}
struct Slot<B: Budget> {
    response: Option<Reply<B>>,
    abandoned: Option<Reply<B>>,
    attempts: usize,
    suppressed: bool,
}
pub(super) struct GroupState<B: Budget> {
    lifecycle: BatchLifecycle,
    slots: Buffer<Slot<B>, B>,
    output: Buffer<Reply<B>, B>,
    batch: bool,
}
pub(super) struct Group<B: Budget> {
    state: Shared<Mutex<GroupState<B>>, B>,
}
impl<B: Budget> Clone for Group<B> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}
impl<B: Budget> Group<B> {
    pub(super) fn new(
        budget: B,
        count: usize,
        batch: bool,
    ) -> Result<Self, ManagedError<B::Error>> {
        let mut slots = Buffer::new(budget.clone());
        let mut output = Buffer::new(budget.clone());
        slots.reserve(count)?;
        output.reserve(count)?;
        for _ in 0..count {
            slots
                .push(Slot {
                    response: None,
                    abandoned: None,
                    attempts: 0,
                    suppressed: false,
                })
                .map_err(|(error, _)| error)?;
        }
        let mut lifecycle = BatchLifecycle::new(count);
        if !batch {
            lifecycle.finish_dispatch();
        }
        Ok(Self {
            state: Shared::new(
                Mutex::new(GroupState {
                    lifecycle,
                    slots,
                    output,
                    batch,
                }),
                &budget,
            )?,
        })
    }
    fn promote(state: &mut GroupState<B>, index: usize) {
        let slot = &mut state.slots.as_mut_slice()[index];
        if slot.attempts == 0 && slot.response.is_none() {
            if let Some(fallback) = slot.abandoned.take() {
                slot.response = Some(fallback);
                state.lifecycle.complete_slot();
            }
        }
    }
    pub(super) fn finish_dispatch(&self) {
        let mut state = self
            .state
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.lifecycle.finish_dispatch() {
            for index in 0..state.slots.len() {
                Self::promote(&mut state, index);
            }
        }
    }
    pub(super) fn complete(&self, index: usize, response: Reply<B>) {
        let previous = {
            let mut state = self
                .state
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.lifecycle.is_emitted() || state.slots.as_slice()[index].response.is_some() {
                return;
            }
            let previous = state.slots.as_mut_slice()[index].abandoned.take();
            state.slots.as_mut_slice()[index].response = Some(response);
            state.lifecycle.complete_slot();
            previous
        };
        drop(previous);
    }
    fn abandon(&self, index: usize, response: Reply<B>) {
        let mut state = self
            .state
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.lifecycle.is_emitted() || state.slots.as_slice()[index].response.is_some() {
            return;
        }
        if !state.batch {
            if !state.slots.as_slice()[index].suppressed {
                state.slots.as_mut_slice()[index].suppressed = true;
                state.lifecycle.complete_slot();
            }
            return;
        }
        if state.slots.as_slice()[index].abandoned.is_none() {
            state.slots.as_mut_slice()[index].abandoned = Some(response);
        }
        if state.lifecycle.is_dispatched() {
            Self::promote(&mut state, index);
        }
    }
    fn begin_attempt(&self, index: usize) {
        let mut state = self
            .state
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.slots.as_mut_slice()[index].attempts += 1;
    }
    fn end_attempt(&self, index: usize) {
        let mut state = self
            .state
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.slots.as_mut_slice()[index].attempts -= 1;
        if state.lifecycle.is_dispatched() {
            Self::promote(&mut state, index);
        }
    }
    pub(super) fn take_ready(&self, budget: B) -> Option<(bool, Buffer<Reply<B>, B>)> {
        let mut state = self
            .state
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.lifecycle.take_completed() {
            return None;
        }
        for index in 0..state.slots.len() {
            if let Some(response) = state.slots.as_mut_slice()[index].response.take() {
                // Output backing was prepaid for all original response slots.
                if state.output.push(response).is_err() {
                    unreachable!("prepaid batch output");
                }
            }
        }
        let output = std::mem::replace(&mut state.output, Buffer::new(budget));
        Some((state.batch, output))
    }
}
/// The one original response destination of an incoming call.
pub struct Responder<B: Budget> {
    group: Group<B>,
    index: usize,
    id: MessageId<B>,
    method: Span<B>,
    generation: u64,
    connection: Weak<Mutex<Core<B>>, B>,
    armed: bool,
    budget: B,
}
impl<B: Budget> Responder<B> {
    pub(super) fn new(
        group: Group<B>,
        index: usize,
        id: MessageId<B>,
        method: Span<B>,
        generation: u64,
        connection: Weak<Mutex<Core<B>>, B>,
        budget: B,
    ) -> Self {
        Self {
            group,
            index,
            id,
            method,
            generation,
            connection,
            armed: true,
            budget,
        }
    }
    /// Borrow the original response ID for native result encoding.
    pub fn id(&self) -> &MessageId<B> { &self.id }
    /// Original request cancellation identity; never a replacement marker.
    pub fn cancellation(&self) -> Result<Cancellation<B>, ManagedError<B::Error>> {
        if let Some(connection) = self.connection.upgrade() {
            connection
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .marker(&self.id, self.generation)
        } else {
            Cancellation::new(&self.budget)
        }
    }
    /// Mark the real duration of a handler attempt, including synchronous errors.
    pub fn begin_attempt(&self) -> Attempt<B> {
        self.group.begin_attempt(self.index);
        Attempt {
            group: self.group.clone(),
            index: self.index,
            connection: self.connection.clone(),
        }
    }
    /// Respond using native-validated encoded output for this exact original ID.
    /// Refusal returns this same responder and payload for recovery.
    pub fn respond(
        mut self,
        response: Envelope<B>,
    ) -> Result<(), (ManagedError<B::Error>, Self, Envelope<B>)> {
        if !matches!(response.kind(), MessageKind::Result | MessageKind::Error)
            || response.id() != Some(&self.id)
            || !response.belongs_to(&self.budget)
        {
            return Err((ManagedError::ForeignResponse, self, response));
        }
        self.group.complete(self.index, Reply::Message(response));
        self.armed = false;
        self.wake();
        Ok(())
    }
    /// Apply the SDK's original unhandled-request response without allocating.
    pub fn unhandled(mut self) {
        self.group.complete(
            self.index,
            Reply::Fault {
                id: self.id.clone(),
                fault: Fault::MethodNotFound,
                method: Some(self.method.clone()),
                original: None,
            },
        );
        self.armed = false;
        self.wake();
    }
    fn wake(&self) {
        if let Some(connection) = self.connection.upgrade() {
            wake_driver(&connection);
        }
    }
}
impl<B: Budget> Drop for Responder<B> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.upgrade() {
            connection
                .get()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .release_marker(&self.id, self.generation);
        }
        if self.armed {
            self.group.abandon(
                self.index,
                Reply::Fault {
                    id: self.id.clone(),
                    fault: Fault::Abandoned,
                    method: Some(self.method.clone()),
                    original: None,
                },
            );
            self.wake();
        }
    }
}
/// Actual handler-attempt lifetime; its drop promotes deferred batch fallback.
pub struct Attempt<B: Budget> {
    group: Group<B>,
    index: usize,
    connection: Weak<Mutex<Core<B>>, B>,
}
impl<B: Budget> Drop for Attempt<B> {
    fn drop(&mut self) {
        self.group.end_attempt(self.index);
        if let Some(connection) = self.connection.upgrade() {
            wake_driver(&connection);
        }
    }
}
