//! Scalar caches retain format labels, full payload costs and original grammar identity.
use super::{
    CompatibilityReason, EncodingDecision, FormatSize, HistoryCodecError, HistoryOrigin,
    HistoryView, LegacySizingError, PreparedHistory, SavedChat, SavedPrompt, size, wire,
};
use crate::model::{
    json::{deferred::DecodeLedger, lifetime::Shared},
    retention::RetentionAuthority,
};
use crate::thread::Entry;
use std::sync::Arc;
mod measurements;
use measurements::Measurements;
#[derive(Clone)]
struct LegacyCost {
    retention: super::retention::LegacyIdentity,
    size: Result<FormatSize, CompatibilityReason>,
}
#[derive(Clone)]
struct Measurement {
    version2: FormatSize,
    resident: usize,
    legacy: Option<LegacyCost>,
}
fn legacy_result(
    result: Result<FormatSize, LegacySizingError>,
) -> Result<Result<FormatSize, CompatibilityReason>, HistoryCodecError> {
    match result {
        Ok(size) => Ok(Ok(size)),
        Err(LegacySizingError::UnsupportedRecord(reason)) => Ok(Err(reason)),
        Err(LegacySizingError::Json(error)) => Err(error.into()),
        Err(LegacySizingError::Formatting(error)) => Err(HistoryCodecError::Formatting(error)),
        Err(LegacySizingError::ArithmeticOverflow) => Err(HistoryCodecError::ArithmeticOverflow),
        Err(LegacySizingError::Retention(error)) => Err(HistoryCodecError::Retention(
            nocterm_core::SharedError::from_error(error),
        )),
    }
}
impl Measurement {
    fn row(
        row: &Entry,
        grammar: Option<&super::retention::LegacyFormat>,
    ) -> Result<Self, HistoryCodecError> {
        Ok(Self {
            version2: wire::entry_measure(row)?,
            resident: size::retained(row)?.max(super::allocations::row(row)?),
            legacy: grammar
                .map(|grammar| {
                    Ok::<LegacyCost, HistoryCodecError>(LegacyCost {
                        retention: grammar
                            .retention()
                            .ok_or(crate::model::retention::RetentionError::MissingAuthority)
                            .map_err(|error| {
                                HistoryCodecError::Retention(nocterm_core::SharedError::from_error(
                                    error,
                                ))
                            })?
                            .clone(),
                        size: legacy_result(grammar.entry(row))?,
                    })
                })
                .transpose()?,
        })
    }
    fn prompt(
        prompt: &SavedPrompt,
        grammar: Option<&super::retention::LegacyFormat>,
    ) -> Result<Self, HistoryCodecError> {
        Ok(Self {
            version2: size::measure(prompt)?,
            resident: size::retained(prompt)?.max(super::allocations::prompt(prompt)?),
            legacy: grammar
                .map(|grammar| {
                    Ok::<LegacyCost, HistoryCodecError>(LegacyCost {
                        retention: grammar
                            .retention()
                            .ok_or(crate::model::retention::RetentionError::MissingAuthority)
                            .map_err(|error| {
                                HistoryCodecError::Retention(nocterm_core::SharedError::from_error(
                                    error,
                                ))
                            })?
                            .clone(),
                        size: legacy_result(grammar.prompt(prompt))?,
                    })
                })
                .transpose()?,
        })
    }
    fn legacy(
        &self,
        grammar: &super::retention::LegacyFormat,
    ) -> Result<FormatSize, HistoryCodecError> {
        let measured = self
            .legacy
            .as_ref()
            .filter(|cost| {
                grammar
                    .retention()
                    .is_some_and(|identity| cost.retention.matches(identity))
            })
            .ok_or(HistoryCodecError::CandidateMismatch)?;
        measured.size.map_err(HistoryCodecError::Compatibility)
    }
}
#[derive(Clone)]
pub struct PreparedBudget {
    pub(super) resident_bytes: usize,
    entries: Shared<Measurements>,
    prompts: Shared<Measurements>,
    pub(super) decision: Option<EncodingDecision>,
}
impl std::fmt::Debug for PreparedBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedBudget")
            .field("resident_bytes", &self.resident_bytes)
            .field("rows", &self.entries.len())
            .field("prompts", &self.prompts.len())
            .finish_non_exhaustive()
    }
}
/// Preserve the policy sum of per-row/prompt maxima. Unique physical graph A
/// is measured separately; taking max(sum(P),sum(H)) here loses mixed shapes.
fn component_resident(chat: &SavedChat) -> Result<usize, HistoryCodecError> {
    let mut shell = super::allocations::chat(chat)?;
    let mut components = 0usize;
    for row in &chat.transcript().entries {
        let physical = super::allocations::row(row)?;
        shell = shell
            .checked_sub(physical)
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        components = components
            .checked_add(size::retained(row)?.max(physical))
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
    }
    for prompt in &chat.composer().queue {
        let physical = super::allocations::prompt(prompt)?;
        shell = shell
            .checked_sub(physical)
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        components = components
            .checked_add(size::retained(prompt)?.max(physical))
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
    }
    let view = chat.view()?;
    let framing = wire::Wire::framing(view.framing(), view.shape());
    components
        .checked_add(size::retained(&framing)?.max(shell))
        .ok_or(HistoryCodecError::ArithmeticOverflow)
}
impl PreparedBudget {
    pub(super) fn new(
        chat: &SavedChat,
        grammar: Option<&super::retention::LegacyFormat>,
    ) -> Result<Self, HistoryCodecError> {
        Self::build(chat, grammar, None)
    }
    pub(super) fn new_in_ledger(
        chat: &SavedChat,
        grammar: Option<&super::retention::LegacyFormat>,
        ledger: &DecodeLedger,
    ) -> Result<Self, HistoryCodecError> {
        Self::build(chat, grammar, Some(ledger))
    }
    fn build(
        chat: &SavedChat,
        grammar: Option<&super::retention::LegacyFormat>,
        ledger: Option<&DecodeLedger>,
    ) -> Result<Self, HistoryCodecError> {
        let resident_bytes = component_resident(chat)?;
        super::check_memory(resident_bytes)?;
        Ok(Self {
            resident_bytes,
            entries: Measurements::rows(
                chat.retention(),
                &chat.transcript().entries,
                grammar,
                ledger,
            )?,
            decision: None,
            prompts: Measurements::prompts(
                chat.retention(),
                &chat.composer().queue,
                grammar,
                ledger,
            )?,
        })
    }
}
/// The original immutable payload is retained solely to bind cached values to it.
#[derive(Clone)]
pub struct PromptBudget {
    prompt: Arc<SavedPrompt>,
    measurement: Measurement,
}
impl PromptBudget {
    pub fn prepare(
        prompt: Arc<SavedPrompt>,
        grammar: Option<&super::retention::LegacyFormat>,
    ) -> Result<Self, HistoryCodecError> {
        let measurement = Measurement::prompt(&prompt, grammar)?;
        Ok(Self {
            prompt,
            measurement,
        })
    }
    pub fn matches(&self, prompt: &SavedPrompt) -> bool {
        std::ptr::eq(self.prompt.as_ref(), prompt)
    }
    pub fn version2(&self) -> FormatSize {
        self.measurement.version2
    }
    pub fn resident_bytes(&self) -> usize {
        self.measurement.resident
    }
}
#[derive(Clone)]
pub struct HistoryBudget {
    retention: RetentionAuthority,
    entries: Vec<Measurement>,
    context: BudgetContext,
}
#[derive(Clone)]
struct BudgetContext {
    origin: HistoryOrigin,
    grammar: Option<super::retention::LegacyFormat>,
}

fn add(left: FormatSize, right: FormatSize) -> Result<FormatSize, HistoryCodecError> {
    Ok(FormatSize {
        encoded_bytes: left
            .encoded_bytes
            .checked_add(right.encoded_bytes)
            .ok_or(HistoryCodecError::ArithmeticOverflow)?,
        structural_bytes: left
            .structural_bytes
            .checked_add(right.structural_bytes)
            .ok_or(HistoryCodecError::ArithmeticOverflow)?,
    })
}
fn compose(
    mut frame: FormatSize,
    rows: impl ExactSizeIterator<Item = Result<FormatSize, HistoryCodecError>>,
    queued: impl ExactSizeIterator<Item = Result<FormatSize, HistoryCodecError>>,
) -> Result<FormatSize, HistoryCodecError> {
    let commas = rows
        .len()
        .saturating_sub(1)
        .checked_add(queued.len().saturating_sub(1))
        .ok_or(HistoryCodecError::ArithmeticOverflow)?;
    frame.encoded_bytes = frame
        .encoded_bytes
        .checked_add(commas)
        .ok_or(HistoryCodecError::ArithmeticOverflow)?;
    for value in rows.chain(queued) {
        frame = add(frame, value?)?;
    }
    Ok(frame)
}
impl HistoryBudget {
    pub fn new(retention: &RetentionAuthority) -> Self {
        Self {
            retention: retention.clone(),
            entries: Vec::new(),
            context: BudgetContext {
                origin: HistoryOrigin::New,
                grammar: None,
            },
        }
    }
    pub fn retention(&self) -> &RetentionAuthority {
        &self.retention
    }
    pub fn restored(retention: &RetentionAuthority, prepared: Option<&PreparedHistory>) -> Self {
        prepared.map_or_else(
            || Self::new(retention),
            |prepared| Self {
                retention: retention.clone(),
                entries: prepared.scalars().entries.as_slice().to_vec(),
                context: BudgetContext {
                    origin: prepared.origin(),
                    grammar: prepared.legacy().cloned(),
                },
            },
        )
    }
    pub fn prepare_snapshot(&self, chat: SavedChat) -> Result<SavedChat, HistoryCodecError> {
        super::codec::prepare_origin(chat, self.context.origin, self.context.grammar.clone(), 0)
    }
    pub fn origin(&self) -> HistoryOrigin {
        self.context.origin
    }
    pub fn legacy(&self) -> Option<&super::retention::LegacyFormat> {
        self.context.grammar.as_ref()
    }
    pub fn resident_bytes(&self) -> Result<usize, HistoryCodecError> {
        self.entries.iter().try_fold(0usize, |sum, row| {
            sum.checked_add(row.resident)
                .ok_or(HistoryCodecError::ArithmeticOverflow)
        })
    }
    pub fn refresh(&mut self, entries: &[Entry], dirty: &[usize]) -> Result<(), HistoryCodecError> {
        let old = self.entries.len();
        self.entries.truncate(entries.len());
        for row in &entries[old.min(entries.len())..] {
            self.entries.push(Measurement::row(row, self.legacy())?);
        }
        for index in dirty.iter().copied().filter(|index| *index < entries.len()) {
            self.entries[index] = Measurement::row(&entries[index], self.legacy())?;
        }
        Ok(())
    }
    /// Read-only capture can occur before the foreground dirty-row cache refresh.
    /// Measure actual borrowed rows with the original format context, never admit
    /// a candidate from stale cached row costs.
    pub fn validate_current(
        &self,
        entries: &[Entry],
        view: HistoryView<'_>,
        prompts: &[&PromptBudget],
    ) -> Result<EncodingDecision, HistoryCodecError> {
        let current = Self {
            retention: self.retention.clone(),
            entries: entries
                .iter()
                .map(|row| Measurement::row(row, self.legacy()))
                .collect::<Result<_, _>>()?,
            context: self.context.clone(),
        };
        current.validate(view, prompts)
    }
    /// Real borrowed candidate payloads and matching cached owners replace unlabelled iterators.
    pub fn validate(
        &self,
        view: HistoryView<'_>,
        prompts: &[&PromptBudget],
    ) -> Result<EncodingDecision, HistoryCodecError> {
        if prompts.len() != view.prompts().len() {
            return Err(HistoryCodecError::CandidateMismatch);
        }
        for (index, budget) in prompts.iter().enumerate() {
            if !view
                .prompts()
                .get(index)
                .is_some_and(|prompt| budget.matches(prompt))
            {
                return Err(HistoryCodecError::CandidateMismatch);
            }
        }
        let rows = &self.entries[self.entries.len().saturating_sub(super::MAX_SAVED_ENTRIES)..];
        if rows.len() != view.entries().len() {
            return Err(HistoryCodecError::CandidateMismatch);
        }
        let frame = wire::Wire::framing(view.framing(), view.shape());
        let mut resident = size::retained(&frame)?;
        resident = resident
            .checked_add(self.resident_bytes()?)
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        for prompt in prompts {
            resident = resident
                .checked_add(prompt.resident_bytes())
                .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        }
        if let Some(grammar) = self.legacy() {
            let retained = grammar.retained_heap_bytes().map_err(|error| match error {
                LegacySizingError::Json(error) => error.into(),
                LegacySizingError::Formatting(error) => HistoryCodecError::Formatting(error),
                LegacySizingError::UnsupportedRecord(reason) => {
                    HistoryCodecError::Compatibility(reason)
                }
                LegacySizingError::ArithmeticOverflow => HistoryCodecError::ArithmeticOverflow,
                LegacySizingError::Retention(error) => {
                    HistoryCodecError::Retention(nocterm_core::SharedError::from_error(error))
                }
            })?;
            resident = resident
                .checked_add(retained)
                .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        }
        let version2 = compose(
            size::measure(&frame)?,
            rows.iter().map(|row| Ok(row.version2)),
            prompts.iter().map(|prompt| Ok(prompt.version2())),
        )?;
        super::select_encoding(self.origin(), resident, Ok(version2), || {
            let grammar = self.legacy().ok_or(HistoryCodecError::CandidateMismatch)?;
            let frame = legacy_result(grammar.framing(view.framing(), view.shape()))?
                .map_err(HistoryCodecError::Compatibility)?;
            compose(
                frame,
                rows.iter().map(|row| row.legacy(grammar)),
                prompts
                    .iter()
                    .map(|prompt| prompt.measurement.legacy(grammar)),
            )
        })
    }
}

impl crate::model::retention::graph::sealed::Source for PreparedBudget {}
impl crate::model::retention::graph::OwnerGraph for PreparedBudget {
    fn count(
        &self,
        count: &mut crate::model::retention::graph::Count<'_, '_>,
    ) -> Result<(), crate::model::retention::RetentionError> {
        self.entries.count(count)?;
        self.prompts.count(count)
    }
    fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        self.entries.cleanup(permit);
        self.prompts.cleanup(permit);
    }

    fn adopt(&self, publication: &mut crate::model::retention::document::Publication<'_>) {
        self.entries.adopt(publication);
        self.prompts.adopt(publication);
    }
}
