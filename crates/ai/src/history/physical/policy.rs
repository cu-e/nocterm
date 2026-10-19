//! Immutable semantic proof built from actual guarded owners before publication.
use super::{PromptSlot, SnapshotOwner, local, wire};
use crate::history::MeasurementError;

#[derive(Clone, Copy)]
pub(super) struct ComponentPolicy {
    retained: usize,
}
impl ComponentPolicy {
    pub(super) fn row(row: &super::RowSlot) -> Result<Self, MeasurementError> {
        Ok(Self {
            retained: local::row(row)?.max(wire::retained_row(row)?),
        })
    }
    pub(super) fn prompt(prompt: &super::PromptSlot) -> Result<Self, MeasurementError> {
        Ok(Self {
            retained: local::prompt(prompt)?.max(wire::retained_prompt(prompt)?),
        })
    }
    pub(super) fn bytes(self) -> usize {
        self.retained
    }
}

pub(super) struct SnapshotPolicy {
    retained: usize,
}
impl SnapshotPolicy {
    pub(super) fn prepare(snapshot: &SnapshotOwner) -> Result<Self, MeasurementError> {
        let mut retained = local::shell(snapshot)?.max(wire::retained_framing(snapshot)?);
        for row in snapshot.rows() {
            retained = add(retained, row.cached_policy()?)?;
        }
        for prompt in snapshot.composer().queue() {
            retained = add(retained, prompt.cached_policy()?)?;
        }
        Ok(Self { retained })
    }
    pub(super) fn bytes(&self) -> usize {
        self.retained
    }
}
pub(super) struct DispatchPolicy {
    retained: usize,
}
impl DispatchPolicy {
    pub(super) fn prepare(prompt: &PromptSlot) -> Result<Self, MeasurementError> {
        // Request-only context/history/session framing contributes its actual
        // prepaid physical storage. It receives no invented history wire tax.
        Ok(Self {
            retained: prompt.cached_policy()?,
        })
    }
    pub(super) fn bytes(&self) -> usize {
        self.retained
    }
}
fn add(left: usize, right: usize) -> Result<usize, MeasurementError> {
    left.checked_add(right)
        .ok_or(MeasurementError::ArithmeticOverflow)
}
