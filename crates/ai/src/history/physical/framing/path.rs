//! Resume path payload participates in the same actual shared bundle topology.
use super::super::{
    bundle::Body,
    slots::{PathGuard, PathSlot},
};
use crate::model::retention::{CensusPermit, RetentionError, document::Publication, graph::Count};
pub(super) struct ResumePath(PathSlot);
impl ResumePath {
    pub(super) fn new(value: PathSlot) -> Self {
        Self(value)
    }
    pub(super) fn read(&self) -> PathGuard<'_> {
        self.0.read()
    }
}
impl Body for ResumePath {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.0.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.0.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.0.adopt(publication);
    }
    fn retire(&self) {
        self.0.retire();
    }
}
