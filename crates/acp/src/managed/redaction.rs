//! Original diagnostic rule semantics with explicit per-source search scratch.
use super::{budget::SourceBudget, copy, error::DecodeError};
use agent_client_protocol::managed::{Buffer, ManagedError, Source};
use nocterm_ai::{
    history::source::NativeAllocation,
    model::json::deferred::JsonDecodeError,
    redact::{Replacement, redaction_rules},
};
use regex_automata::{
    Input,
    nfa::thompson::pikevm::{
        PikeVM,
        managed::{self, ManagedCache},
    },
};
use std::alloc::Layout;
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error(transparent)]
    Pattern(Box<regex_automata::nfa::thompson::BuildError>),
    #[error("diagnostic rule unexpectedly matches empty text")]
    EmptyRule,
}
impl From<regex_automata::nfa::thompson::BuildError> for BuildError {
    fn from(error: regex_automata::nfa::thompson::BuildError) -> Self {
        Self::Pattern(Box::new(error))
    }
}
#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error(transparent)]
    Decode(#[from] DecodeError<JsonDecodeError>),
    #[error(transparent)]
    Native(#[from] ManagedError<JsonDecodeError>),
    #[error(transparent)]
    Search(#[from] managed::Error<JsonDecodeError>),
    #[error("invalid diagnostic capture offsets")]
    Captures,
}
impl managed::Budget for SourceBudget {
    type Credit = NativeAllocation;
    type Error = JsonDecodeError;
    fn reserve(&self, layout: Layout) -> Result<Self::Credit, Self::Error> {
        self.context().reserve_native(layout)
    }
}
/// Compiled constant rule infrastructure is injected once; every search cache
/// belongs to its original source, and is never parked in a shared/global pool.
pub(crate) struct DiagnosticRedactor {
    rules: [PikeVM; 5],
}
impl DiagnosticRedactor {
    pub(crate) fn new() -> Result<Self, BuildError> {
        let [a, b, c, d, e] = redaction_rules();
        let rules = [
            PikeVM::new(a.pattern())?,
            PikeVM::new(b.pattern())?,
            PikeVM::new(c.pattern())?,
            PikeVM::new(d.pattern())?,
            PikeVM::new(e.pattern())?,
        ];
        if rules.iter().any(|rule| rule.get_nfa().has_empty()) {
            return Err(BuildError::EmptyRule);
        }
        Ok(Self { rules })
    }
    pub(super) fn scrub(
        &self,
        mut source: Source<SourceBudget>,
        secrets: &[Source<SourceBudget>],
        budget: &SourceBudget,
    ) -> Result<Source<SourceBudget>, Error> {
        if !source.belongs_to(budget) || secrets.iter().any(|secret| !secret.belongs_to(budget)) {
            return Err(ManagedError::InvalidEnvelope.into());
        }
        for secret in secrets {
            source = literal(source, secret.text(), budget)?;
        }
        for (re, rule) in self.rules.iter().zip(redaction_rules()) {
            source = stage(re, rule.replacement(), source, budget)?;
        }
        Ok(source)
    }
    pub(super) fn owned(
        &self,
        source: Source<SourceBudget>,
        secrets: &[Source<SourceBudget>],
        budget: &SourceBudget,
    ) -> Result<nocterm_ai::model::native::NativeOwned<String>, Error> {
        let source = self.scrub(source, secrets, budget)?;
        Ok(copy::text(budget, source.text())?)
    }
}
fn literal(
    source: Source<SourceBudget>,
    secret: &str,
    budget: &SourceBudget,
) -> Result<Source<SourceBudget>, Error> {
    if !source.text().contains(secret) {
        return Ok(source);
    }
    let mut output = Source::buffer(budget.clone());
    let mut copied = 0;
    for (start, matched) in source.text().match_indices(secret) {
        output.extend_bytes(&source.text().as_bytes()[copied..start])?;
        output.extend_bytes(b"[REDACTED]")?;
        copied = start + matched.len();
    }
    output.extend_bytes(&source.text().as_bytes()[copied..])?;
    Ok(Source::from_buffer(output)?)
}
fn capture(
    cache: &ManagedCache<'_, SourceBudget>,
    number: usize,
    text: &str,
    output: &mut Buffer<u8, SourceBudget>,
) -> Result<(), Error> {
    if let (Some(start), Some(end)) = (
        cache.slots().get(number * 2).copied().flatten(),
        cache.slots().get(number * 2 + 1).copied().flatten(),
    ) {
        let value = text.get(start.get()..end.get()).ok_or(Error::Captures)?;
        output.extend_bytes(value.as_bytes())?;
    }
    Ok(())
}
fn stage(
    re: &PikeVM,
    replacement: Replacement,
    source: Source<SourceBudget>,
    budget: &SourceBudget,
) -> Result<Source<SourceBudget>, Error> {
    let mut cache = ManagedCache::new(re, budget.clone())?;
    let mut output = Source::buffer(budget.clone());
    let mut copied = 0;
    while cache
        .search(&Input::new(source.text()).span(copied..source.text().len()))?
        .is_some()
    {
        let (start, end) = match (cache.slots()[0], cache.slots()[1]) {
            (Some(a), Some(b)) => (a.get(), b.get()),
            _ => return Err(Error::Captures),
        };
        if end <= start {
            return Err(Error::Captures);
        }
        output.extend_bytes(
            source
                .text()
                .get(copied..start)
                .ok_or(Error::Captures)?
                .as_bytes(),
        )?;
        let suffix = match replacement {
            Replacement::Literal(value) => value,
            Replacement::CaptureOne(value) => {
                capture(&cache, 1, source.text(), &mut output)?;
                value
            }
            Replacement::CaptureOneTwo(value) => {
                capture(&cache, 1, source.text(), &mut output)?;
                capture(&cache, 2, source.text(), &mut output)?;
                value
            }
        };
        output.extend_bytes(suffix.as_bytes())?;
        copied = end;
    }
    if copied == 0 {
        return Ok(source);
    }
    output.extend_bytes(&source.text().as_bytes()[copied..])?;
    Ok(Source::from_buffer(output)?)
}
