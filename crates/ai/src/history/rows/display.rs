//! Pure client-display construction with authentic text/JSON allocation ownership.
use super::{DecodeLedger, JsonDecodeError, Reservation, size_of};
use crate::model::shared_display::SharedDisplay;
use crate::model::{
    controls::Presence,
    json::deferred::{DecodedJson, literal::LiteralText},
    tools::ClientDisplay,
};
/// Literal payload remains controlled until moved into the final display owner.
pub enum DecodedDisplayOutcome {
    Pending,
    Succeeded(DecodedJson),
    Failed(LiteralText),
}
/// Arc/display drops before its actual backing reservation. No arbitrary caller
/// ClientDisplay or mutable JSON can be adopted as an authentic managed receipt.
pub struct DecodedDisplay {
    display: SharedDisplay,
    backing: Reservation,
    _fixed: Reservation,
}
impl DecodedDisplay {
    pub fn new(
        version: u8,
        tool: LiteralText,
        arguments: DecodedJson,
        destination: Presence<LiteralText>,
        redacted: Presence<bool>,
        outcome: Presence<DecodedDisplayOutcome>,
        ledger: &DecodeLedger,
    ) -> Result<Self, JsonDecodeError> {
        let fixed = ledger.reserve(size_of::<Self>())?;
        use crate::model::native::NativeOwned;
        let tool = super::owned_text(tool, ledger)?;
        let arguments = super::owned_json(arguments, ledger)?;
        let destination = match destination {
            Presence::Absent => NativeOwned::not_supplied(ledger)?,
            Presence::Null => NativeOwned::null(ledger)?,
            Presence::Value(text) => super::owned_text(text, ledger)?.presence(),
        };
        let outcome = match outcome {
            Presence::Absent => NativeOwned::not_supplied(ledger)?,
            Presence::Null => NativeOwned::null(ledger)?,
            Presence::Value(DecodedDisplayOutcome::Pending) => {
                crate::model::native::tools::pending(ledger)?.presence()
            }
            Presence::Value(DecodedDisplayOutcome::Succeeded(value)) => {
                crate::model::native::tools::succeeded(super::owned_json(value, ledger)?).presence()
            }
            Presence::Value(DecodedDisplayOutcome::Failed(text)) => {
                crate::model::native::tools::failed(super::owned_text(text, ledger)?).presence()
            }
        };
        let display = SharedDisplay::admitted(crate::model::native::tools::display(
            version,
            tool,
            arguments,
            destination,
            redacted,
            outcome,
        )?)?;
        let backing = ledger.reserve(0)?;
        Ok(Self {
            display,
            backing,
            _fixed: fixed,
        })
    }
    pub fn as_display(&self) -> &ClientDisplay {
        &self.display
    }
    pub(crate) fn into_parts(
        self,
        ledger: &DecodeLedger,
    ) -> Result<(SharedDisplay, Reservation), JsonDecodeError> {
        if !self.backing.belongs_to(ledger) {
            return Err(JsonDecodeError::ForeignLedger);
        }
        Ok((self.display, self.backing))
    }
}
#[cfg(test)]
mod tests;
