//! Snapshot facts have no event IDs or invented chain positions.
use super::{MAX_PROJECTION_ENTRIES, ProjectionBinding, ProjectionError, RecordKind};
use crate::{
    coordinator::CoordinatorEvent,
    identity::{ChatControlRevision, Timestamp},
    journal::{HeadAnchor, HeadAnchorKind},
};

#[derive(Clone)]
pub struct SnapshotFact {
    pub control: ChatControlRevision,
    pub time: Timestamp,
    pub event: CoordinatorEvent,
}
/// Actual authenticated outer snapshot anchor and a sparse semantic display window.
/// Restoring this is a separate transition from replaying individual events.
#[derive(Clone)]
pub struct SnapshotBaseline {
    binding: ProjectionBinding,
    anchor: HeadAnchor,
    control: ChatControlRevision,
    time: Timestamp,
    omitted_earlier: bool,
    facts: Vec<SnapshotFact>,
}
impl SnapshotBaseline {
    pub fn new(
        binding: ProjectionBinding,
        anchor: HeadAnchor,
        control: ChatControlRevision,
        time: Timestamp,
        omitted_earlier: bool,
        facts: Vec<SnapshotFact>,
    ) -> Result<Self, ProjectionError> {
        if anchor.kind() != HeadAnchorKind::Snapshot
            || anchor.position().writer() > binding.writer()
            || facts.len() > MAX_PROJECTION_ENTRIES
        {
            return Err(ProjectionError::InvalidRecord(RecordKind::SnapshotBaseline));
        }
        let mut previous = ChatControlRevision::default();
        let mut previous_time = Timestamp::from_millis(0);
        for fact in &facts {
            if fact.control < previous
                || fact.control > control
                || fact.time < previous_time
                || fact.time > time
            {
                return Err(ProjectionError::InvalidRecord(RecordKind::SnapshotBaseline));
            }
            previous = fact.control;
            previous_time = fact.time;
        }
        Ok(Self {
            binding,
            anchor,
            control,
            time,
            omitted_earlier,
            facts,
        })
    }
    pub fn binding(&self) -> &ProjectionBinding {
        &self.binding
    }
    pub fn anchor(&self) -> &HeadAnchor {
        &self.anchor
    }
    pub fn control(&self) -> ChatControlRevision {
        self.control
    }
    pub fn time(&self) -> Timestamp {
        self.time
    }
    pub fn omitted_earlier(&self) -> bool {
        self.omitted_earlier
    }
    pub fn facts(&self) -> &[SnapshotFact] {
        &self.facts
    }
}
