//! App-requested launch arguments are real source-owned strings before spawn.
use super::budget::SourceBudget;
use agent_client_protocol::managed::{Buffer, ManagedError, Source};
use nocterm_ai::model::json::deferred::JsonDecodeError;
use std::path::Path;
type Error = ManagedError<JsonDecodeError>;
pub(super) struct Arguments {
    values: Buffer<Source<SourceBudget>, SourceBudget>,
    budget: SourceBudget,
}
impl Arguments {
    pub(super) fn new(budget: SourceBudget) -> Self {
        Self {
            values: Buffer::new(budget.clone()),
            budget,
        }
    }
    pub(super) fn text(&mut self, text: &str) -> Result<(), Error> {
        self.source(Source::copy(text, self.budget.clone())?)
    }
    pub(super) fn source(&mut self, value: Source<SourceBudget>) -> Result<(), Error> {
        self.values.push(value).map_err(|(error, _)| error)
    }
    #[cfg(unix)]
    pub(super) fn path(&mut self, path: &Path) -> Result<(), Error> {
        let mut bytes = Source::buffer(self.budget.clone());
        let mut remaining = path.as_os_str().as_encoded_bytes();
        loop {
            match std::str::from_utf8(remaining) {
                Ok(valid) => {
                    bytes.extend_bytes(valid.as_bytes())?;
                    break;
                }
                Err(error) => {
                    bytes.extend_bytes(&remaining[..error.valid_up_to()])?;
                    bytes.extend_bytes("\u{fffd}".as_bytes())?;
                    match error.error_len() {
                        Some(len) => remaining = &remaining[error.valid_up_to() + len..],
                        None => break,
                    }
                }
            }
        }
        self.source(Source::from_buffer(bytes)?)
    }
    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = &str> + Clone {
        self.values.as_slice().iter().map(Source::text)
    }
}
