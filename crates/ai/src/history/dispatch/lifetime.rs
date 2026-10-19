//! Immutable active aliases pin the same original dispatch loan until the last consumer ends.
use super::{DispatchError, DispatchLease};
use crate::{
    history::physical::{AttachmentSlot, DispatchPayload, LiveAttachmentSlot},
    model::{
        json::lifetime::Shared,
        retention::document::{AllocationPlan, PendingAllocation},
    },
};

struct DispatchBody {
    original: DispatchLease,
    _header: PendingAllocation,
}

/// Private and unique until publication; no consumer can alias an unpublished loan.
pub(in crate::history) struct DispatchLifetime(Shared<DispatchBody>);
impl DispatchLifetime {
    pub(in crate::history) fn new(original: DispatchLease) -> Result<Self, DispatchError> {
        let header = original
            .producer
            .0
            .scope
            .reserve_phase_metadata(AllocationPlan::shared::<DispatchBody>())?;
        Ok(Self(Shared::new(DispatchBody {
            original,
            _header: header,
        })))
    }
    pub(in crate::history) fn original_mut(&mut self) -> Result<&mut DispatchLease, DispatchError> {
        self.0
            .get_mut()
            .map(|body| &mut body.original)
            .ok_or(DispatchError::Busy)
    }
    pub(in crate::history) fn activate(self) -> ActiveDispatch {
        ActiveDispatch(self.0)
    }
}

/// The native request owns one alias. Scope consumers can only retain the same loan.
pub struct ActiveDispatch(Shared<DispatchBody>);
impl ActiveDispatch {
    pub fn payload(&self) -> Option<&DispatchPayload> {
        self.0.original.payload()
    }
    pub fn scope(&self) -> ActivePromptScope {
        ActivePromptScope(self.0.clone())
    }
}

/// Fixed active-turn input authority ends when the original turn releases this alias.
/// It cannot modify, replace or manufacture a dispatch payload.
#[derive(Clone)]
pub struct ActivePromptScope(Shared<DispatchBody>);
impl ActivePromptScope {
    pub fn live_attachments(&self) -> Option<&[LiveAttachmentSlot]> {
        self.0.original.payload()?.live_attachments()
    }
    pub fn saved_attachments(&self) -> &[AttachmentSlot] {
        self.0
            .original
            .payload()
            .map_or(&[], |payload| payload.prompt().attachments())
    }
}
