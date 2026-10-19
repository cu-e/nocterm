//! Original last64KiB diagnostic bytes stay prepaid across native source close.
use super::budget::SourceBudget;
use agent_client_protocol::managed::{Buffer, ManagedError, Source};
use nocterm_ai::model::json::deferred::JsonDecodeError;
type Error = ManagedError<JsonDecodeError>;
const LIMIT: usize = 64 * 1024;
pub(super) struct Tail {
    bytes: Buffer<u8, SourceBudget>,
    head: usize,
    budget: SourceBudget,
}
impl Tail {
    pub(super) fn new(budget: SourceBudget) -> Self {
        Self {
            bytes: Buffer::new(budget.clone()),
            head: 0,
            budget,
        }
    }
    /// Input is already in the actual native reader's prepaid inline scratch.
    /// Growth is admitted before copying; overwrite has no hidden temporary Vec.
    pub(super) fn push(&mut self, input: &[u8]) -> Result<(), Error> {
        let input = if input.len() > LIMIT {
            &input[input.len() - LIMIT..]
        } else {
            input
        };
        let growth = input.len().min(LIMIT - self.bytes.len());
        self.bytes.reserve_amortized(growth)?;
        let (new, overwrite) = input.split_at(growth);
        for &byte in new {
            self.bytes.push(byte).map_err(|(error, _)| error)?;
        }
        for &byte in overwrite {
            self.bytes.as_mut_slice()[self.head] = byte;
            self.head = (self.head + 1) % LIMIT;
        }
        Ok(())
    }
    /// This output remains unredacted and is never itself published as an event.
    /// The native error owner must pass it through the paid original redactor.
    pub(super) fn decoded(&self) -> Result<Source<SourceBudget>, Error> {
        let mut ordered = Source::buffer(self.budget.clone());
        ordered.extend_bytes(&self.bytes.as_slice()[self.head..])?;
        ordered.extend_bytes(&self.bytes.as_slice()[..self.head])?;
        let mut decoded = Source::buffer(self.budget.clone());
        let mut rest = ordered.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    decoded.extend_bytes(valid.as_bytes())?;
                    break;
                }
                Err(error) => {
                    let (valid, invalid) = rest.split_at(error.valid_up_to());
                    decoded.extend_bytes(valid)?;
                    decoded.extend_bytes("\u{fffd}".as_bytes())?;
                    match error.error_len() {
                        Some(length) => rest = &invalid[length..],
                        None => break,
                    }
                }
            }
        }
        Source::from_buffer(decoded)
    }
}
