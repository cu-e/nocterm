//! Named Source failure tickets never substitute a fresh allocation authority.
use super::SourceContext;
use crate::model::{
    diagnostic::{RemoteFault, RemoteFaultKind},
    failure::{ManagedNativeFailure, NativeErrorSlot},
    json::deferred::NativeAllocationError,
    native::NativeOwned,
};
use std::{error::Error, fmt};
/// Source-owned preadmission, prepared before constructing a concrete error.
pub struct SourceErrorSlot<E: Error + Send + Sync + 'static> {
    slot: NativeErrorSlot<E>,
    source: SourceContext,
}
impl<E: Error + Send + Sync + 'static> SourceErrorSlot<E> {
    pub(super) fn prepare(
        source: &SourceContext,
        nested_error_bytes: usize,
    ) -> Result<Self, NativeAllocationError> {
        Ok(Self {
            slot: NativeErrorSlot::prepare(source.ledger(), nested_error_bytes)?,
            source: source.clone(),
        })
    }
    pub fn capture(self, error: E) -> ManagedNativeFailure {
        let Self { slot, source } = self;
        let captured = slot.capture(error);
        drop(source);
        captured
    }
    pub fn capture_with_nested(self, error: E, actual_nested_bytes: usize) -> ManagedNativeFailure {
        let Self { slot, source } = self;
        let captured = slot.capture_with_nested(error, actual_nested_bytes);
        drop(source);
        captured
    }
}
/// Managed commands retain their actual paid diagnosis. The adapter produces
/// redacted bounded remote text before it crosses the domain boundary.
pub enum ManagedCommandError {
    Remote(NativeOwned<RemoteFault>),
    Native(ManagedNativeFailure),
    Admission(NativeAllocationError),
    /// Process shutdown could not be confirmed; the process may still run.
    CleanupUnconfirmed(ManagedNativeFailure),
    LocalRestoreUnsupported {
        fork: bool,
    },
}
impl ManagedCommandError {
    pub fn authentication_required(&self) -> bool {
        matches!(self, Self::Remote(fault) if fault.get().kind() == RemoteFaultKind::AuthenticationRequired)
    }
    pub fn cleanup_unconfirmed(&self) -> bool {
        matches!(self, Self::CleanupUnconfirmed(_))
    }
    pub fn restore_unavailable(&self) -> bool {
        matches!(self, Self::LocalRestoreUnsupported { .. })
            || matches!(self, Self::Remote(fault) if fault.get().kind() == RemoteFaultKind::RestoreUnavailable)
    }
}
impl fmt::Debug for ManagedCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Remote(value) => f.debug_tuple("Remote").field(value.get()).finish(),
            Self::Native(value) => f.debug_tuple("Native").field(value).finish(),
            Self::Admission(value) => f.debug_tuple("Admission").field(value).finish(),
            Self::CleanupUnconfirmed(value) => {
                f.debug_tuple("CleanupUnconfirmed").field(value).finish()
            }
            Self::LocalRestoreUnsupported { fork } => f
                .debug_struct("LocalRestoreUnsupported")
                .field("fork", fork)
                .finish(),
        }
    }
}
impl fmt::Display for ManagedCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Remote(value) => match value.get().kind() {
                RemoteFaultKind::AuthenticationRequired => {
                    write!(f, "Authentication required: {}", value.get())
                }
                RemoteFaultKind::Rpc | RemoteFaultKind::RestoreUnavailable => {
                    write!(f, "Agent error: {}", value.get())
                }
            },
            Self::Native(value) | Self::CleanupUnconfirmed(value) => fmt::Display::fmt(value, f),
            Self::Admission(value) => fmt::Display::fmt(value, f),
            Self::LocalRestoreUnsupported { fork: true } => {
                f.write_str("The agent cannot fork chats.")
            }
            Self::LocalRestoreUnsupported { fork: false } => {
                f.write_str("The agent cannot reopen earlier chats.")
            }
        }
    }
}
impl Error for ManagedCommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Remote(value) => Some(value.get()),
            Self::Native(value) | Self::CleanupUnconfirmed(value) => Some(value),
            Self::Admission(value) => Some(value),
            Self::LocalRestoreUnsupported { .. } => None,
        }
    }
}
