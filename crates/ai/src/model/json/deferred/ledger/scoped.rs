//! The concrete operation key prepays the ledger header and every new backing.
//! Neither ledger nor credit metadata retains the document account strongly.
use super::JsonDecodeError;
use crate::model::retention::{
    RetentionError,
    document::{
        AllocationPlan, ConstructionKey, ConstructionRole, DocumentRetention, PendingAllocation,
        Publication,
    },
    graph::{Count, OwnerGraph},
};

#[derive(Debug)]
pub(super) struct ScopedLedger {
    key: ConstructionKey,
    header: PendingAllocation,
    endpoint_pending: bool,
}
impl ScopedLedger {
    pub(super) fn new(
        role: &ConstructionRole,
        header_bytes: usize,
    ) -> Result<Self, JsonDecodeError> {
        let key = role.construction_key().map_err(error)?;
        let header = key
            .reserve_pending(AllocationPlan::checked_bytes(header_bytes).map_err(error)?)
            .map_err(error)?;
        Ok(Self {
            key,
            header,
            endpoint_pending: false,
        })
    }
    pub(super) fn for_source(
        role: &ConstructionRole,
        header_bytes: usize,
    ) -> Result<Self, JsonDecodeError> {
        let key = role.construction_key().map_err(error)?;
        let header = role
            .scope()
            .reserve_phase_metadata(AllocationPlan::checked_bytes(header_bytes).map_err(error)?)
            .map_err(error)?;
        Ok(Self {
            key,
            header,
            endpoint_pending: true,
        })
    }
    pub(super) fn header_paid_here(&self, count: &Count<'_, '_>) -> bool {
        self.endpoint_pending && count.pays_pending_header(&self.header)
    }
    pub(super) fn reserve_policy(
        &self,
        bytes: usize,
    ) -> Result<crate::model::retention::document::PolicyLiability, JsonDecodeError> {
        if !self.endpoint_pending {
            return Err(JsonDecodeError::Retention(RetentionError::WrongPayload));
        }
        self.key.reserve_policy(bytes).map_err(error)
    }
    pub(super) fn reserve(&self, bytes: usize) -> Result<PendingAllocation, RetentionError> {
        self.key
            .reserve_pending(AllocationPlan::checked_bytes(bytes)?)
    }
    pub(super) fn belongs_to(&self, scope: &DocumentRetention) -> bool {
        self.key.belongs_to(scope)
    }
    pub(super) fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        if self.endpoint_pending {
            self.header.validate_endpoint_header_for_count(count)?;
        } else {
            self.header.validate_for_count(count)?;
        }
        self.key.count(count)
    }
    pub(super) fn cleanup(&self, permit: &crate::model::retention::CensusPermit<'_>) {
        self.header.cleanup_for_count(permit);
        self.key.cleanup(permit);
    }
    pub(super) fn adopt(&self, publication: &mut Publication<'_>) {
        if !self.endpoint_pending {
            self.header.adopt_under(publication);
        }
        self.key.adopt(publication);
    }
}
pub(super) fn error(error: RetentionError) -> JsonDecodeError {
    match error {
        RetentionError::Arithmetic => JsonDecodeError::Arithmetic,
        RetentionError::Limit {
            requested,
            available,
        } => JsonDecodeError::Limit {
            requested,
            available,
        },
        other => JsonDecodeError::Retention(other),
    }
}
