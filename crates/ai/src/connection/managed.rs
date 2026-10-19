//! Original native events cross the service port with their actual paid owners.
use crate::{
    history::source::{ManagedNotice, ManagedPermission, NativeAllocation, SourceContext},
    model::{auth::PermissionResponder, json::deferred::JsonDecodeError, native::NativeOwned},
};
use std::{
    alloc::Layout,
    task::{Context, Poll},
};

pub enum ManagedAgentEvent {
    Barrier(ManagedEventBarrier),
    Failed(crate::history::source::ManagedCommandError),
    Session(ManagedNotice),
    Permission {
        request: ManagedPermission,
        respond: PermissionResponder,
    },
    Exited {
        code: Option<i32>,
        stderr_tail: NativeOwned<String>,
    },
}
/// Poll the original adapter channel without allocating an erased receive future.
pub trait ManagedEventSource: Send + 'static {
    fn poll_event(&mut self, cx: &mut Context<'_>) -> Poll<Option<ManagedAgentEvent>>;
}
/// Non-cloneable foreground receiver. Its concrete source allocation is freed
/// before its original native allocation credit, even when never polled.
pub struct ManagedEventReceiver {
    source: Box<dyn ManagedEventSource>,
    _allocation: NativeAllocation,
    _lifetime: Option<crate::history::source::SourceLifetime>,
}
impl ManagedEventReceiver {
    pub fn new<S: ManagedEventSource>(
        context: &SourceContext,
        source: S,
    ) -> Result<Self, JsonDecodeError> {
        Self::prepare(context, source, None)
    }
    pub fn with_lifetime<S: ManagedEventSource>(
        context: &SourceContext,
        lifetime: crate::history::source::SourceLifetime,
        source: S,
    ) -> Result<Self, JsonDecodeError> {
        Self::prepare(context, source, Some(lifetime))
    }
    fn prepare<S: ManagedEventSource>(
        context: &SourceContext,
        source: S,
        lifetime: Option<crate::history::source::SourceLifetime>,
    ) -> Result<Self, JsonDecodeError> {
        let input = ReceiverConstruction { source, lifetime };
        if input
            .lifetime
            .as_ref()
            .is_some_and(|lifetime| !context.same_source(lifetime.context_ref()))
        {
            return Err(JsonDecodeError::ForeignLedger);
        }
        let allocation = context.reserve_native(Layout::new::<S>())?;
        Ok(Self {
            source: Box::new(input.source),
            _allocation: allocation,
            _lifetime: input.lifetime,
        })
    }
    pub fn poll_event(&mut self, cx: &mut Context<'_>) -> Poll<Option<ManagedAgentEvent>> {
        self.source.poll_event(cx)
    }
}
struct ReceiverConstruction<S> {
    source: S,
    lifetime: Option<crate::history::source::SourceLifetime>,
}
/// The original paid adapter fence decides acknowledgement and abandonment.
pub trait ManagedEventAcknowledgement: Send + 'static {
    fn acknowledge(&mut self);
}
pub struct ManagedEventBarrier {
    original: Box<dyn ManagedEventAcknowledgement>,
    _allocation: NativeAllocation,
}
impl ManagedEventBarrier {
    pub fn new<F: ManagedEventAcknowledgement>(
        context: &SourceContext,
        original: F,
    ) -> Result<Self, JsonDecodeError> {
        let allocation = context.reserve_native(Layout::new::<F>())?;
        Ok(Self {
            original: Box::new(original),
            _allocation: allocation,
        })
    }
    pub fn acknowledge(mut self) {
        self.original.acknowledge();
    }
}

mod commands;
pub use commands::{
    ManagedAgentCommands, ManagedAgentConnection, ManagedCommandsHandle, ManagedPreparedPrompt,
};

mod startup;
pub use startup::ManagedStartupHandle;

mod workspace;
pub use workspace::ManagedWorkspaceHandle;

mod cleanup;
pub use cleanup::{ManagedProcessCleanup, ManagedShutdownResult, ManagedShutdownWaiter};
