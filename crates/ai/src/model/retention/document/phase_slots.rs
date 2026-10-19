//! Fixed physical phase roots belong to the four actual construction producers.
use super::{Publication, phase::PhaseIdentity, roles::ConstructionKind};
use crate::model::retention::{
    CensusPermit, RetentionError,
    graph::{Count, OwnerGraph, sealed},
};
#[derive(Default)]
struct PhaseSlot {
    current: Option<PhaseIdentity>,
    retiring: Option<PhaseIdentity>,
}
pub(super) struct PhaseRoles {
    foreground: PhaseSlot,
    writer: PhaseSlot,
    reader: PhaseSlot,
    source: PhaseSlot,
}
impl PhaseRoles {
    pub(super) fn new() -> Self {
        Self {
            foreground: PhaseSlot::default(),
            writer: PhaseSlot::default(),
            reader: PhaseSlot::default(),
            source: PhaseSlot::default(),
        }
    }
    fn slot(&self, kind: ConstructionKind) -> &PhaseSlot {
        match kind {
            ConstructionKind::Foreground => &self.foreground,
            ConstructionKind::Writer => &self.writer,
            ConstructionKind::Reader => &self.reader,
            ConstructionKind::Source => &self.source,
        }
    }
    fn slot_mut(&mut self, kind: ConstructionKind) -> &mut PhaseSlot {
        match kind {
            ConstructionKind::Foreground => &mut self.foreground,
            ConstructionKind::Writer => &mut self.writer,
            ConstructionKind::Reader => &mut self.reader,
            ConstructionKind::Source => &mut self.source,
        }
    }
    pub(super) fn replace(&mut self, kind: ConstructionKind, next: Option<PhaseIdentity>) {
        let slot = self.slot_mut(kind);
        assert!(slot.retiring.is_none(), "synchronous phase retirement");
        slot.retiring = slot.current.take();
        slot.current = next;
    }
    pub(super) fn retiring(&self, kind: ConstructionKind) -> Option<&PhaseIdentity> {
        self.slot(kind).retiring.as_ref()
    }
    pub(super) fn take_retiring(&mut self, kind: ConstructionKind) -> Option<PhaseIdentity> {
        self.slot_mut(kind).retiring.take()
    }
    pub(super) fn count_except(
        &self,
        excluded: Option<ConstructionKind>,
        count: &mut Count<'_, '_>,
    ) -> Result<(), RetentionError> {
        for kind in [
            ConstructionKind::Foreground,
            ConstructionKind::Writer,
            ConstructionKind::Reader,
            ConstructionKind::Source,
        ] {
            let slot = self.slot(kind);
            if let Some(current) = &slot.current {
                current.count(count)?;
            }
            if excluded != Some(kind)
                && let Some(retiring) = &slot.retiring
            {
                retiring.count(count)?;
            }
        }
        Ok(())
    }
}
impl sealed::Source for PhaseRoles {}
impl OwnerGraph for PhaseRoles {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.count_except(None, count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        for kind in [
            ConstructionKind::Foreground,
            ConstructionKind::Writer,
            ConstructionKind::Reader,
            ConstructionKind::Source,
        ] {
            let slot = self.slot(kind);
            if let Some(current) = &slot.current {
                current.cleanup(permit);
            }
            if let Some(retiring) = &slot.retiring {
                retiring.cleanup(permit);
            }
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        for kind in [
            ConstructionKind::Foreground,
            ConstructionKind::Writer,
            ConstructionKind::Reader,
            ConstructionKind::Source,
        ] {
            let slot = self.slot(kind);
            if let Some(current) = &slot.current {
                current.adopt(publication);
            }
            if let Some(retiring) = &slot.retiring {
                retiring.adopt(publication);
            }
        }
    }
}
