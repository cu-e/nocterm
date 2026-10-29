//! Sub-admission inside the actual caller reservation, never a new quota owner.
use nocterm_cloud_model::projection::ProjectionError;
use nocterm_core::allocation::AllocationLease;

pub(super) struct DecodeBudget<'a> {
    allocation: &'a AllocationLease,
    retained: usize,
    scratch: usize,
}
impl<'a> DecodeBudget<'a> {
    pub(super) fn new(allocation: &'a AllocationLease) -> Result<Self, ProjectionError> {
        if allocation.payloads() == 0 {
            return Err(ProjectionError::Capacity);
        }
        Ok(Self {
            allocation,
            retained: 0,
            scratch: 0,
        })
    }
    pub(super) fn admit(&mut self, bytes: usize) -> Result<(), ProjectionError> {
        let retained = self
            .retained
            .checked_add(bytes)
            .ok_or(ProjectionError::Capacity)?;
        self.require(retained, self.scratch)?;
        self.retained = retained;
        Ok(())
    }
    pub(super) fn with_scratch(
        allocation: &'a AllocationLease,
        scratch: usize,
    ) -> Result<Self, ProjectionError> {
        let mut budget = Self::new(allocation)?;
        budget.require(0, scratch)?;
        budget.scratch = scratch;
        Ok(budget)
    }
    fn require(&self, retained: usize, scratch: usize) -> Result<(), ProjectionError> {
        if retained > nocterm_cloud_model::projection::MAX_REPLAY_BYTES
            || retained
                .checked_add(scratch)
                .is_none_or(|total| total > self.allocation.bytes())
        {
            Err(ProjectionError::Capacity)
        } else {
            Ok(())
        }
    }
    fn during_scratch<T>(
        &mut self,
        bytes: usize,
        work: impl FnOnce(&mut Self) -> Result<T, ProjectionError>,
    ) -> Result<T, ProjectionError> {
        let previous = self.scratch;
        let next = previous
            .checked_add(bytes)
            .ok_or(ProjectionError::Capacity)?;
        self.require(self.retained, next)?;
        self.scratch = next;
        let result = work(self);
        self.scratch = previous;
        result
    }
    pub(super) fn json(&mut self, text: &str) -> Result<serde_json::Value, ProjectionError> {
        let scratch = text.len().checked_mul(2).ok_or(ProjectionError::Capacity)?;
        self.during_scratch(scratch, |budget| {
            let bytes = nocterm_ai::history::structural_bytes(text.as_bytes()).map_err(
                |error| match error {
                    nocterm_ai::history::HistoryCodecError::StructuralLimit => {
                        ProjectionError::Capacity
                    }
                    _ => ProjectionError::InvalidRecord,
                },
            )?;
            budget.admit(bytes)?;
            serde_json::from_str(text).map_err(|_| ProjectionError::InvalidRecord)
        })
    }
}
