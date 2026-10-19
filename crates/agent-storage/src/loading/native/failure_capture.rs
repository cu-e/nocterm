//! A prospective final native failure preserves the original display projection.
use super::{Error, route::RouteError};
use crate::loading::{held_file::HeldReadError, literal_reader::LiteralReadError};
use nocterm_ai::{
    history::HistoryCodecError,
    model::{
        failure::NativeErrorSlot,
        json::deferred::{DecodeLedger, JsonDecodeError, NativeAllocationError},
    },
    storage::AgentStorageError,
};

#[derive(Debug, thiserror::Error)]
enum Projection {
    #[error("{0}")]
    Held(#[source] HeldReadError),
    #[error("{0}")]
    Error(#[source] Error),
    #[error("{0}")]
    Build(#[source] JsonDecodeError),
}
pub(super) struct FailureCapture {
    slot: NativeErrorSlot<Projection>,
}
impl FailureCapture {
    pub(super) fn prepare(ledger: &DecodeLedger) -> Result<Self, AgentStorageError> {
        Ok(Self {
            slot: NativeErrorSlot::prepare(ledger, 0)
                .map_err(AgentStorageError::ManagedAdmission)?,
        })
    }
    pub(super) fn held(self, error: HeldReadError, id: &str) -> AgentStorageError {
        if let Some(admission) = admission(&error) {
            return AgentStorageError::ManagedAdmission(admission);
        }
        match error {
            HeldReadError::ManagedReplay(error) => error,
            HeldReadError::Admission(error) => AgentStorageError::ManagedAdmission(error),
            HeldReadError::ManagedNative(error)
                if error.as_ref().kind() == std::io::ErrorKind::NotFound =>
            {
                AgentStorageError::ManagedNotFound(error)
            }
            HeldReadError::ManagedNative(error) => AgentStorageError::ManagedRead(error),
            HeldReadError::Native { source, .. }
                if source.as_ref().kind() == std::io::ErrorKind::NotFound =>
            {
                AgentStorageError::NotFound(id.into())
            }
            HeldReadError::Native { path, source, .. } => AgentStorageError::Read { path, source },
            #[cfg(test)]
            HeldReadError::Json(error) => HistoryCodecError::Json(error).into(),
            HeldReadError::EncodedLimit => HistoryCodecError::EncodedLimit.into(),
            HeldReadError::StructuralLimit => HistoryCodecError::StructuralLimit.into(),
            HeldReadError::Arithmetic => HistoryCodecError::ArithmeticOverflow.into(),
            error => {
                HistoryCodecError::ManagedValidation(self.slot.capture(Projection::Held(error)))
                    .into()
            }
        }
    }
    pub(super) fn storage(self, error: Error, id: &str) -> AgentStorageError {
        if let Some(admission) = admission(&error) {
            return AgentStorageError::ManagedAdmission(admission);
        }
        match error {
            Error::Held(error) | Error::Route(RouteError::Held(error)) => self.held(error, id),
            Error::Build(JsonDecodeError::Limit { .. }) => {
                HistoryCodecError::StructuralLimit.into()
            }
            Error::Build(JsonDecodeError::Arithmetic) => {
                HistoryCodecError::ArithmeticOverflow.into()
            }
            Error::Build(error) => {
                HistoryCodecError::ManagedValidation(self.slot.capture(Projection::Build(error)))
                    .into()
            }
            Error::Codec(error) | Error::Route(RouteError::Codec(error)) => error.into(),
            Error::Literal(LiteralReadError::ManagedJson(error))
            | Error::Route(RouteError::ManagedJson(error)) => {
                HistoryCodecError::ManagedJson(error).into()
            }
            error => {
                HistoryCodecError::ManagedValidation(self.slot.capture(Projection::Error(error)))
                    .into()
            }
        }
    }
}
fn admission(mut error: &(dyn std::error::Error + 'static)) -> Option<NativeAllocationError> {
    loop {
        if let Some(admission) = error.downcast_ref::<NativeAllocationError>() {
            return Some(*admission);
        }
        error = error.source()?;
    }
}
