//! Redacted remote faults preserve original precedence, caps and restore meaning.
use super::{
    budget::SourceBudget,
    copy,
    error::DecodeError,
    redaction::{self, DiagnosticRedactor},
    value::Value,
};
use agent_client_protocol::managed::{Buffer, Envelope, Source};
use nocterm_ai::{
    history::source::ManagedCommandError,
    model::{diagnostic::RemoteFaultKind, json::deferred::JsonDecodeError, native::NativeOwned},
};
use std::sync::Arc;
#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error(transparent)]
    Decode(#[from] DecodeError<JsonDecodeError>),
    #[error(transparent)]
    Redaction(#[from] redaction::Error),
    #[error(transparent)]
    Allocation(#[from] agent_client_protocol::managed::ManagedError<JsonDecodeError>),
    #[error(transparent)]
    Native(#[from] JsonDecodeError),
}
pub(super) struct Diagnostics {
    rules: Arc<DiagnosticRedactor>,
    secrets: Buffer<Source<SourceBudget>, SourceBudget>,
    budget: SourceBudget,
}
fn prefix(text: &str, count: usize) -> &str {
    let end = text
        .char_indices()
        .nth(count)
        .map_or(text.len(), |(index, _)| index);
    &text[..end]
}
fn detail(value: Value<'_>) -> Result<Option<NativeOwned<String>>, Error> {
    if value.raw().starts_with('"') {
        return Ok(Some(value.text()?));
    }
    if !value.raw().starts_with('{') {
        return Ok(None);
    }
    let [message, details, error] = value.fields(["message", "details", "error"])?;
    for candidate in [message, details, error.clone()].into_iter().flatten() {
        if candidate.raw().starts_with('"') {
            return Ok(Some(candidate.text()?));
        }
    }
    if let Some(error) = error.filter(|value| value.raw().starts_with('{')) {
        let [message] = error.fields(["message"])?;
        if let Some(message) = message.filter(|value| value.raw().starts_with('"')) {
            return Ok(Some(message.text()?));
        }
    }
    Ok(None)
}
/// For ASCII needles, per-scalar Unicode lowercase has the same substring
/// result as str::to_lowercase: contextual Greek sigma cannot produce ASCII.
fn restore_unavailable(message: &str) -> bool {
    [
        "session not found",
        "unknown session",
        "session does not exist",
        "no rollout found for thread id ",
    ]
    .into_iter()
    .any(|needle| {
        message.char_indices().any(|(start, _)| {
            message[start..]
                .chars()
                .flat_map(char::to_lowercase)
                .take(needle.len())
                .eq(needle.chars())
        })
    })
}
impl Diagnostics {
    pub(super) fn new(
        rules: Arc<DiagnosticRedactor>,
        secrets: Buffer<Source<SourceBudget>, SourceBudget>,
        budget: SourceBudget,
    ) -> Self {
        Self {
            rules,
            secrets,
            budget,
        }
    }
    pub(super) fn owned(&self, source: Source<SourceBudget>) -> Result<NativeOwned<String>, Error> {
        Ok(self
            .rules
            .owned(source, self.secrets.as_slice(), &self.budget)?)
    }
    pub(super) fn remote(
        &self,
        envelope: &Envelope<SourceBudget>,
        restoring: bool,
    ) -> Result<ManagedCommandError, Error> {
        let span = envelope
            .payload()
            .supplied()
            .ok_or(DecodeError::Field("remote fault"))?;
        let root = Value::from_span(span, &self.budget)?;
        let [code, message, data] = root.fields(["code", "message", "data"])?;
        let code = Value::required(code, "fault code")?
            .raw()
            .parse::<i32>()
            .map_err(|_| DecodeError::Field("fault code"))?;
        let message = Value::required(message, "fault message")?.text()?;
        let mut combined = Source::buffer(self.budget.clone());
        combined.extend_bytes(message.as_str().as_bytes())?;
        if let Some(data) = data
            && let Some(detail) = detail(data)?
        {
            let source = Source::copy(detail.as_str(), self.budget.clone())?;
            let detail = self
                .rules
                .scrub(source, self.secrets.as_slice(), &self.budget)?;
            combined.extend_bytes(b": ")?;
            combined.extend_bytes(prefix(detail.text(), 2048).as_bytes())?;
        }
        drop(message);
        let redacted = self.rules.scrub(
            Source::from_buffer(combined)?,
            self.secrets.as_slice(),
            &self.budget,
        )?;
        let kind = if code == -32000 {
            RemoteFaultKind::AuthenticationRequired
        } else if restoring
            && (code == -32601 || restore_unavailable(prefix(redacted.text(), 4096)))
        {
            RemoteFaultKind::RestoreUnavailable
        } else {
            RemoteFaultKind::Rpc
        };
        let message = copy::text(&self.budget, prefix(redacted.text(), 4096))?;
        Ok(ManagedCommandError::Remote(
            nocterm_ai::model::native::remote::remote_fault(code, kind, message)?,
        ))
    }
}

#[cfg(test)]
mod original;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod lowercase_tests {
    use super::restore_unavailable;
    #[test]
    fn allocation_free_restore_matching_matches_original_unicode_lowercase() {
        for prefix in ["", "ΣΟΣ", "İ", "ſ", "Ω", "Ａ", "\u{0307}"] {
            for message in [
                "session not found",
                "SESSION NOT FOUND",
                "Unknown Session",
                "Session does not exist",
                "no rollout found for thread id abc",
                "unknown sessionx",
                "unknown sessİon",
                "normal",
            ] {
                let text = format!("{prefix}{message}Σ");
                let lower = text.to_lowercase();
                let expected = [
                    "session not found",
                    "unknown session",
                    "session does not exist",
                    "no rollout found for thread id ",
                ]
                .iter()
                .any(|needle| lower.contains(needle));
                assert_eq!(restore_unavailable(&text), expected, "{text}");
            }
        }
    }
}
