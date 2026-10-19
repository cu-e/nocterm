//! Read-only observations of actual ledger topology for native lifetime tests.
use crate::model::{
    json::deferred::DecodeLedger,
    retention::{
        CensusPermit, RetentionError,
        document::Publication,
        graph::{self, Count, OwnerGraph, sealed},
    },
};
struct LedgerPair<'a>(&'a DecodeLedger, &'a DecodeLedger);
impl sealed::Source for LedgerPair<'_> {}
impl OwnerGraph for LedgerPair<'_> {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.0.count(count)?;
        self.1.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.0.cleanup(permit);
        self.1.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.0.adopt(publication);
        self.1.adopt(publication);
    }
}
/// Actual old-only ledger/phase/account topology relative to the current ledger.
/// The observation pins no owner after returning. Tests must drop their own old
/// ledger handle before measuring the last real error or request destruction.
pub fn unique_ledger_metadata(
    old: &DecodeLedger,
    current: &DecodeLedger,
) -> Result<usize, RetentionError> {
    if !old.authority().same(current.authority()) {
        return Err(RetentionError::ForeignAuthority);
    }
    let both = graph::measure(old.authority(), &LedgerPair(old, current))?;
    let kept = graph::measure(current.authority(), current)?;
    both.checked_sub(kept).ok_or(RetentionError::Arithmetic)
}

/// Test-only original Writer construction under its unchanged document limit.
/// A foreign role cannot mint this fixture; no callback or limit override exists.
pub fn writer_ledger(
    role: &crate::model::retention::document::ConstructionRole,
) -> Result<DecodeLedger, crate::model::json::deferred::JsonDecodeError> {
    if !role.is_writer() {
        return Err(crate::model::json::deferred::JsonDecodeError::Retention(
            RetentionError::WrongPayload,
        ));
    }
    DecodeLedger::for_operation(role, crate::history::MAX_RESIDENT_BYTES)
}
