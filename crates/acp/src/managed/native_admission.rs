//! Primitive allocation refusals stay inline across concrete native error owners.
#[cfg(unix)]
use super::environment_snapshot;
use super::{diagnostics, error::DecodeError, native_process, redaction};
use agent_client_protocol::managed::ManagedError;
use nocterm_ai::model::json::deferred::{JsonDecodeError, NativeAllocationError};
pub(super) fn json(error: &JsonDecodeError) -> Option<NativeAllocationError> {
    match error {
        JsonDecodeError::Arithmetic => Some(NativeAllocationError::Arithmetic),
        JsonDecodeError::Limit {
            requested,
            available,
        } => Some(NativeAllocationError::Limit {
            requested: *requested,
            available: *available,
        }),
        JsonDecodeError::Retention(error) => Some(NativeAllocationError::Retention(*error)),
        _ => None,
    }
}
pub(super) fn sdk(error: &ManagedError<JsonDecodeError>) -> Option<NativeAllocationError> {
    match error {
        ManagedError::Refused(error) => json(error),
        ManagedError::CapacityOverflow => Some(NativeAllocationError::Arithmetic),
        _ => None,
    }
}
pub(super) fn decode(error: &DecodeError<JsonDecodeError>) -> Option<NativeAllocationError> {
    match error {
        DecodeError::Allocation(error) => sdk(error),
        DecodeError::Native(error) => json(error),
        DecodeError::Measurement(error) => measurement(error),
        DecodeError::Model(nocterm_ai::model::diagnostic::ModelError::JsonArithmetic) => {
            Some(NativeAllocationError::Arithmetic)
        }
        DecodeError::Model(nocterm_ai::model::diagnostic::ModelError::Retention(error)) => {
            Some(NativeAllocationError::Retention(*error.as_ref()))
        }
        _ => None,
    }
}
fn measurement(error: &nocterm_ai::history::MeasurementError) -> Option<NativeAllocationError> {
    match error {
        nocterm_ai::history::MeasurementError::Retention(error) => {
            Some(NativeAllocationError::Retention(*error))
        }
        nocterm_ai::history::MeasurementError::ArithmeticOverflow => {
            Some(NativeAllocationError::Arithmetic)
        }
        _ => None,
    }
}
pub(super) fn semantic<T>(
    error: &nocterm_ai::history::source::SourceSemanticRefusal<T>,
) -> Option<NativeAllocationError> {
    match error.cause() {
        nocterm_ai::history::source::SourceSemanticError::Allocation(error) => json(error),
        nocterm_ai::history::source::SourceSemanticError::Measurement(error) => measurement(error),
        _ => None,
    }
}
fn redaction(error: &redaction::Error) -> Option<NativeAllocationError> {
    match error {
        redaction::Error::Decode(error) => decode(error),
        redaction::Error::Native(error) => sdk(error),
        redaction::Error::Search(
            regex_automata::nfa::thompson::pikevm::managed::Error::Refused(error),
        ) => json(error),
        _ => None,
    }
}
pub(super) fn diagnostic(error: &diagnostics::Error) -> Option<NativeAllocationError> {
    match error {
        diagnostics::Error::Decode(error) => decode(error),
        diagnostics::Error::Redaction(error) => redaction(error),
        diagnostics::Error::Allocation(error) => sdk(error),
        diagnostics::Error::Native(error) => json(error),
    }
}
pub(super) fn process(error: &native_process::Error) -> Option<NativeAllocationError> {
    match error {
        #[cfg(unix)]
        native_process::Error::Snapshot(environment_snapshot::Error::Native(error)) => sdk(error),
        native_process::Error::Allocation(error) => sdk(error),
        native_process::Error::Native(error)
        | native_process::Error::Path(nocterm_ai::model::native::NativePathError::Admission(
            error,
        )) => json(error),
        native_process::Error::Workspace(crate::workspace::managed::Cause::Admission(error)) => {
            Some(*error)
        }
        native_process::Error::Redaction(error) => redaction(error),
        _ => None,
    }
}

pub(super) fn processing(error: &DecodeError<JsonDecodeError>) -> bool {
    matches!(
        error,
        DecodeError::Field(_)
            | DecodeError::Syntax(_)
            | DecodeError::InvalidPath
            | DecodeError::Model(
                nocterm_ai::model::diagnostic::ModelError::Unsupported(_)
                    | nocterm_ai::model::diagnostic::ModelError::ExtensionLimit
                    | nocterm_ai::model::diagnostic::ModelError::ReservedMetadata
            )
    )
}
