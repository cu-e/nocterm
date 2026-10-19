//! Closed actual payload alternatives for the original six retained slots.
use crate::{
    history::physical::{DispatchPayload, DispatchProjection, SnapshotOwner, SnapshotProjection},
    model::retention::{
        CensusPermit, RetentionError,
        graph::{Count, OwnerGraph, sealed},
    },
};

pub(super) enum RetainedPayload {
    Snapshot(SnapshotOwner),
    Dispatch(DispatchPayload),
}
impl RetainedPayload {
    pub(super) fn snapshot(&self) -> Option<&SnapshotOwner> {
        match self {
            Self::Snapshot(value) => Some(value),
            Self::Dispatch(_) => None,
        }
    }
    pub(super) fn dispatch(&self) -> Option<&DispatchPayload> {
        match self {
            Self::Dispatch(value) => Some(value),
            Self::Snapshot(_) => None,
        }
    }
}
#[derive(Clone)]
pub(super) enum RetainedProjection {
    Snapshot(SnapshotProjection),
    Dispatch(DispatchProjection),
}
impl sealed::Source for RetainedProjection {}
impl OwnerGraph for RetainedProjection {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        match self {
            Self::Snapshot(value) => value.count(count),
            Self::Dispatch(value) => value.count(count),
        }
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        match self {
            Self::Snapshot(value) => value.cleanup(permit),
            Self::Dispatch(value) => value.cleanup(permit),
        }
    }
    fn adopt(&self, publication: &mut super::Publication<'_>) {
        match self {
            Self::Snapshot(value) => value.adopt(publication),
            Self::Dispatch(value) => value.adopt(publication),
        }
    }
}

pub(super) struct DispatchRoots<'a> {
    live: &'a SnapshotProjection,
    dispatch: &'a DispatchProjection,
}
impl<'a> DispatchRoots<'a> {
    pub(super) fn new(live: &'a SnapshotProjection, dispatch: &'a DispatchProjection) -> Self {
        Self { live, dispatch }
    }
}
impl sealed::Source for DispatchRoots<'_> {}
impl OwnerGraph for DispatchRoots<'_> {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.live.count(count)?;
        self.dispatch.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.live.cleanup(permit);
        self.dispatch.cleanup(permit);
    }
    fn adopt(&self, publication: &mut super::Publication<'_>) {
        self.live.adopt(publication);
        self.dispatch.adopt(publication);
    }
}
