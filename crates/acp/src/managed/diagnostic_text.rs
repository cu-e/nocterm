//! Formatting native causes writes directly into the original paid diagnostic.
use super::budget::SourceBudget;
use agent_client_protocol::managed::{Buffer, ManagedError, Source};
use nocterm_ai::model::json::deferred::JsonDecodeError;
use std::fmt::{self, Write as _};
struct Text {
    bytes: Buffer<u8, SourceBudget>,
    refusal: Option<ManagedError<JsonDecodeError>>,
}
impl fmt::Write for Text {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.bytes.extend_bytes(text.as_bytes()).map_err(|error| {
            self.refusal = Some(error);
            fmt::Error
        })
    }
}
pub(super) fn format(
    arguments: fmt::Arguments<'_>,
    budget: &SourceBudget,
) -> Result<Source<SourceBudget>, ManagedError<JsonDecodeError>> {
    let mut text = Text {
        bytes: Source::buffer(budget.clone()),
        refusal: None,
    };
    if text.write_fmt(arguments).is_err() {
        return Err(text.refusal.unwrap_or(ManagedError::InvalidEnvelope));
    }
    Source::from_buffer(text.bytes)
}
