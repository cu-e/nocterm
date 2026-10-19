//! Paid shutdown observation reuses the original command and native attempt engine.
use super::ManagedCommandsHandle;
use crate::{
    history::source::{
        ManagedCommandError, ManagedSourceFuture, NativeAllocation, SourceContext, SourceLifetime,
    },
    model::{
        failure::ManagedNativeFailure,
        json::{deferred::NativeAllocationError, lifetime::Shared},
    },
};
use std::{alloc::Layout, sync::Mutex};
mod attempt;
#[cfg(test)]
mod tests;
mod waiters;
pub use attempt::ManagedShutdownWaiter;
use attempt::ShutdownAttempt;
pub type ManagedShutdownResult = Result<(), ManagedNativeFailure>;
struct CleanupBody {
    commands: ManagedCommandsHandle,
    state: Mutex<CleanupState>,
    _allocation: NativeAllocation,
    _lifetime: SourceLifetime,
}
struct CleanupState {
    current: ShutdownAttempt,
    final_selected: bool,
}
/// Clones share the same process cleanup decision and allocate nothing.
#[derive(Clone)]
pub struct ManagedProcessCleanup(Shared<CleanupBody>);
impl ManagedProcessCleanup {
    pub fn new(
        commands: ManagedCommandsHandle,
        lifetime: SourceLifetime,
    ) -> Result<Self, NativeAllocationError> {
        let context = commands.source_context().clone();
        if !context.same_source(lifetime.context_ref()) {
            return Err(crate::model::retention::RetentionError::ForeignAuthority.into());
        }
        let allocation = header::<CleanupBody>(&context)?;
        let current =
            ShutdownAttempt::prepare(commands.clone(), &context, lifetime.clone(), false)?;
        Ok(Self(Shared::new(CleanupBody {
            commands,
            state: Mutex::new(CleanupState {
                current,
                final_selected: false,
            }),
            _allocation: allocation,
            _lifetime: lifetime,
        })))
    }
    pub fn commands(&self) -> &ManagedCommandsHandle {
        &self.0.commands
    }
    pub fn owns(&self, commands: &ManagedCommandsHandle) -> bool {
        self.0.commands.same_connection(commands)
    }
    pub fn join(&self) -> Result<ManagedShutdownWaiter, NativeAllocationError> {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .current
            .join()
    }
    pub fn capture(&self) -> Result<ManagedShutdownWaiter, NativeAllocationError> {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.final_selected && state.current.failed() {
            let replacement = ShutdownAttempt::prepare(
                self.0.commands.clone(),
                self.0._allocation.source(),
                self.0._lifetime.clone(),
                true,
            )?;
            // Register the actual final observer before retiring the old failure.
            let waiter = replacement.join()?;
            state.current = replacement;
            state.final_selected = true;
            Ok(waiter)
        } else {
            state.current.join()
        }
    }
}
fn header<T>(context: &SourceContext) -> Result<NativeAllocation, NativeAllocationError> {
    context.reserve_requested(
        Layout::from_size_align(
            crate::model::allocations::shared_inline_bytes::<T>(),
            std::mem::align_of::<T>().max(std::mem::align_of::<usize>()),
        )
        .map_err(|_| NativeAllocationError::Arithmetic)?,
    )
}
