//! Borrowed kept-root projection for genuine suffix retirement. No owned header
//! can fall out of a slot before its suffix census/credit transfer succeeds.
use super::{
    DocumentState,
    roles::{ConstructionKind, RetainedKind},
};
use crate::model::retention::{
    CensusPermit, RetentionError,
    graph::{Count, OwnerGraph, sealed},
};
pub(super) struct Kept<'a> {
    state: &'a DocumentState,
    retiring: Option<RetainedKind>,
    source: Option<ConstructionKind>,
    phase: Option<ConstructionKind>,
}
impl<'a> Kept<'a> {
    pub(super) fn phase(state: &'a DocumentState, kind: ConstructionKind) -> Self {
        Self {
            state,
            retiring: None,
            source: None,
            phase: Some(kind),
        }
    }
    pub(super) fn retiring(state: &'a DocumentState, kind: RetainedKind) -> Self {
        Self {
            state,
            retiring: Some(kind),
            source: None,
            phase: None,
        }
    }
    pub(super) fn source(state: &'a DocumentState, kind: ConstructionKind) -> Self {
        Self {
            state,
            retiring: None,
            source: Some(kind),
            phase: None,
        }
    }
}
impl sealed::Source for Kept<'_> {}
impl OwnerGraph for Kept<'_> {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.state.identity.count(count)?;
        self.state
            .roles
            .count_except(self.retiring, self.source, count)?;
        self.state.phases.count_except(self.phase, count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.state.identity.cleanup(permit);
        self.state.roles.cleanup(permit);
        self.state.phases.cleanup(permit);
    }
    fn adopt(&self, _publication: &mut super::Publication<'_>) { /* Retirement-only view owns no allocation token. */
    }
}
