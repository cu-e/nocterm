//! Prospective native preflight errors keep their original concrete guard.
use super::HeldReadError;
use crate::loading::literal_reader::LiteralReadError;
use nocterm_ai::{
    history::PreflightFailure,
    model::{failure::NativeErrorSlot, json::deferred::DecodeLedger},
};

pub(super) enum PreparedLiteralFailure {
    Legacy,
    Managed(NativeErrorSlot<LiteralReadError>),
}
impl PreparedLiteralFailure {
    pub(super) fn prepare(ledger: &DecodeLedger, managed: bool) -> Result<Self, HeldReadError> {
        if managed {
            NativeErrorSlot::prepare(ledger, 0)
                .map(Self::Managed)
                .map_err(HeldReadError::Admission)
        } else {
            Ok(Self::Legacy)
        }
    }
    pub(super) fn capture(self, error: LiteralReadError) -> HeldReadError {
        match error {
            LiteralReadError::Preflight(PreflightFailure::StructuralLimit) => {
                HeldReadError::StructuralLimit
            }
            LiteralReadError::Preflight(PreflightFailure::ArithmeticOverflow) => {
                HeldReadError::Arithmetic
            }
            error => match self {
                Self::Managed(slot) => HeldReadError::ManagedLiteral(slot.capture(error)),
                Self::Legacy => {
                    HeldReadError::Literal(nocterm_core::SharedError::from_error(error))
                }
            },
        }
    }
}
