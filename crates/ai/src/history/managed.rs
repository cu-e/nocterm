//! Exact format selection over real physical and native owners, without DTO copies.
use super::{
    EncodingDecision, HistoryCodecError, HistoryOrigin, PreparedHistory, SavedDocument,
    physical::{SnapshotOwner, SnapshotWire},
};
use crate::model::native::NativeOwned;

fn add(left: usize, right: usize) -> Result<usize, HistoryCodecError> {
    left.checked_add(right)
        .ok_or(HistoryCodecError::ArithmeticOverflow)
}
/// Each component has its own policy maximum. Shared graph A is deduplicated
/// separately; it is never subtracted from independently counted components.
pub fn decision_managed(snapshot: &SnapshotOwner) -> Result<EncodingDecision, HistoryCodecError> {
    let policy = snapshot
        .policy_bytes()
        .map_err(super::MeasurementError::from)?;
    let resident = policy.max(
        snapshot
            .retained_bytes()
            .map_err(super::MeasurementError::from)?,
    );
    let prepared = snapshot.prepared();
    super::select_encoding(
        prepared
            .as_ref()
            .map_or(HistoryOrigin::New, PreparedHistory::origin),
        resident,
        super::measure_json(&SnapshotWire::new(snapshot)),
        || {
            let grammar = prepared
                .as_ref()
                .and_then(PreparedHistory::legacy)
                .ok_or(HistoryCodecError::CandidateMismatch)?;
            super::codec::legacy_size(grammar.managed_measure(snapshot))
        },
    )
}

/// Native archive materialization is still owned by its original writer ledger.
pub fn decision_document(
    owner: &NativeOwned<SavedDocument>,
    prepared: &PreparedHistory,
) -> Result<EncodingDecision, HistoryCodecError> {
    let document = owner.get();
    let mut shell = super::allocations::document(document)?;
    let mut components = 0;
    for row in &document.transcript.entries {
        let physical = super::allocations::row(row)?;
        shell = shell
            .checked_sub(physical)
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        components = add(components, physical.max(super::size::retained(row)?))?;
    }
    for prompt in &document.composer.queue {
        let physical = super::allocations::prompt(prompt)?;
        shell = shell
            .checked_sub(physical)
            .ok_or(HistoryCodecError::ArithmeticOverflow)?;
        components = add(components, physical.max(super::size::retained(prompt)?))?;
    }
    let view = document.view()?;
    let framing = super::wire::Wire::framing(view.framing(), view.shape());
    let resident = add(components, shell.max(super::size::retained(&framing)?))?;
    super::codec::decide(view, Some(prepared), resident)
}
