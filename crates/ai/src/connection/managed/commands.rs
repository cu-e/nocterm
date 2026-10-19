//! Native command authority shares one prepaid concrete implementation.
use super::{super::CloseSessionOutcome, ManagedEventReceiver};
use crate::{
    history::{
        dispatch::ActiveDispatch,
        physical::DispatchPayload,
        source::{
            ManagedAgentInfo, ManagedCommandError, ManagedDispatchFuture, ManagedOpened,
            ManagedSourceFuture, NativeAllocation, SourceContext,
        },
    },
    model::{
        controls::ConfigOption,
        ids::{AuthMethodId, SessionId},
        json::{deferred::NativeAllocationError, lifetime::Shared},
        native::NativeOwned,
        session::{
            ConfigurationSelection, ModeSelection, PromptOutcome, RestoreSession, SessionOpen,
        },
    },
};
use std::alloc::Layout;
pub(super) type Operation<T> =
    Result<ManagedSourceFuture<Result<T, ManagedCommandError>>, NativeAllocationError>;
/// Staging borrows requests synchronously; returned work owns only paid data.
pub trait ManagedAgentCommands: Send + Sync + 'static {
    fn new_session(&self, request: &SessionOpen) -> Operation<ManagedOpened>;
    fn restore_session(&self, request: &RestoreSession) -> Operation<ManagedOpened>;
    fn prepare_prompt(
        &self,
        payload: &DispatchPayload,
    ) -> Result<ManagedPreparedPrompt, ManagedCommandError>;
    fn cancel(&self, session: &SessionId) -> Result<(), ManagedCommandError>;
    fn set_mode(&self, request: &ModeSelection) -> Operation<()>;
    fn set_config_option(
        &self,
        request: &ConfigurationSelection,
    ) -> Operation<NativeOwned<Vec<ConfigOption>>>;
    fn authenticate(&self, method: &AuthMethodId) -> Operation<()>;
    fn close_session(&self, session: &SessionId) -> Operation<CloseSessionOutcome>;
    fn shutdown_gracefully(&self) -> Operation<()>;
    fn shutdown_finally(&self) -> Operation<()>;
    fn shutdown(&self);
}
struct Body {
    concrete: Box<dyn ManagedAgentCommands>,
    _concrete_allocation: NativeAllocation,
    _header: NativeAllocation,
    _lifetime: Option<crate::history::source::SourceLifetime>,
}
struct CommandConstruction<T> {
    concrete: T,
    lifetime: Option<crate::history::source::SourceLifetime>,
}
/// An authenticated alias of one connection. Cloning never allocates.
#[derive(Clone)]
pub struct ManagedCommandsHandle(Shared<Body>);
impl ManagedCommandsHandle {
    pub fn new<T: ManagedAgentCommands>(
        context: &SourceContext,
        concrete: T,
    ) -> Result<Self, NativeAllocationError> {
        Self::prepare(context, concrete, None)
    }
    pub fn new_with_lifetime<T: ManagedAgentCommands>(
        context: &SourceContext,
        lifetime: crate::history::source::SourceLifetime,
        concrete: T,
    ) -> Result<Self, NativeAllocationError> {
        Self::prepare(context, concrete, Some(lifetime))
    }
    fn prepare<T: ManagedAgentCommands>(
        context: &SourceContext,
        concrete: T,
        lifetime: Option<crate::history::source::SourceLifetime>,
    ) -> Result<Self, NativeAllocationError> {
        let input = CommandConstruction { concrete, lifetime };
        if input
            .lifetime
            .as_ref()
            .is_some_and(|lifetime| !context.same_source(lifetime.context_ref()))
        {
            return Err(crate::model::retention::RetentionError::ForeignAuthority.into());
        }
        let concrete_allocation = context.reserve_requested(Layout::new::<T>())?;
        let header = context.reserve_requested(
            Layout::from_size_align(
                crate::model::allocations::shared_inline_bytes::<Body>(),
                std::mem::align_of::<Body>().max(std::mem::align_of::<usize>()),
            )
            .map_err(|_| NativeAllocationError::Arithmetic)?,
        )?;
        Ok(Self(Shared::new(Body {
            concrete: Box::new(input.concrete),
            _concrete_allocation: concrete_allocation,
            _header: header,
            _lifetime: input.lifetime,
        })))
    }
    pub fn same_connection(&self, other: &Self) -> bool {
        std::ptr::eq(self.0.get(), other.0.get())
    }
    pub fn new_session(&self, request: &SessionOpen) -> Operation<ManagedOpened> {
        self.0.concrete.new_session(request)
    }
    pub fn restore_session(&self, request: &RestoreSession) -> Operation<ManagedOpened> {
        self.0.concrete.restore_session(request)
    }
    pub fn prepare_prompt(
        &self,
        payload: &DispatchPayload,
    ) -> Result<ManagedPreparedPrompt, ManagedCommandError> {
        self.0.concrete.prepare_prompt(payload)
    }
    pub fn cancel(&self, session: &SessionId) -> Result<(), ManagedCommandError> {
        self.0.concrete.cancel(session)
    }
    pub fn set_mode(&self, request: &ModeSelection) -> Operation<()> {
        self.0.concrete.set_mode(request)
    }
    pub fn set_config_option(
        &self,
        request: &ConfigurationSelection,
    ) -> Operation<NativeOwned<Vec<ConfigOption>>> {
        self.0.concrete.set_config_option(request)
    }
    pub fn authenticate(&self, method: &AuthMethodId) -> Operation<()> {
        self.0.concrete.authenticate(method)
    }
    pub fn close_session(&self, session: &SessionId) -> Operation<CloseSessionOutcome> {
        self.0.concrete.close_session(session)
    }
    pub(super) fn source_context(&self) -> &SourceContext {
        self.0._header.source()
    }
    pub fn prepare_cleanup_failure(
        &self,
    ) -> Result<crate::history::source::SourceErrorSlot<ManagedCommandError>, NativeAllocationError>
    {
        self.0._header.source().prepare_error(0)
    }
    pub fn shutdown_gracefully(&self) -> Operation<()> {
        self.0.concrete.shutdown_gracefully()
    }
    pub fn shutdown_finally(&self) -> Operation<()> {
        self.0.concrete.shutdown_finally()
    }
    pub fn shutdown(&self) {
        self.0.concrete.shutdown();
    }
}
/// The original nonclone information certificate reaches the foreground once.
pub struct ManagedAgentConnection {
    info: ManagedAgentInfo,
    commands: ManagedCommandsHandle,
    events: ManagedEventReceiver,
}
impl ManagedAgentConnection {
    pub fn new(
        info: ManagedAgentInfo,
        commands: ManagedCommandsHandle,
        events: ManagedEventReceiver,
    ) -> Self {
        Self {
            info,
            commands,
            events,
        }
    }
    pub fn commands(&self) -> &ManagedCommandsHandle {
        &self.commands
    }
    pub fn into_parts(
        self,
    ) -> (
        ManagedAgentInfo,
        ManagedCommandsHandle,
        ManagedEventReceiver,
    ) {
        (self.info, self.commands, self.events)
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

/// Already admitted wire, correlation storage and unpolled native work. Binding
/// the committed original Dispatch loan creates no allocation or publication.
pub struct ManagedPreparedPrompt {
    work: ManagedSourceFuture<Result<NativeOwned<PromptOutcome>, ManagedCommandError>>,
}
impl ManagedPreparedPrompt {
    pub fn new(
        work: ManagedSourceFuture<Result<NativeOwned<PromptOutcome>, ManagedCommandError>>,
    ) -> Self {
        Self { work }
    }
    pub fn bind(
        self,
        dispatch: ActiveDispatch,
    ) -> ManagedDispatchFuture<Result<NativeOwned<PromptOutcome>, ManagedCommandError>> {
        ManagedDispatchFuture::new(self.work, dispatch)
    }
}
