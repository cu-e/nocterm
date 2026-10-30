//! Explicit session execution and finite metadata/cleanup collaborators.
use super::run_session::session_descriptor_bytes;
use nocterm_cloud_model::{
    coordinator::{CoordinatorError, RunSessionBroker},
    journal::{
        JournalCatalogBinding, PublicationAllocation, PublicationWorkBudget, RunSessionFactCleanup,
    },
};
use nocterm_core::allocation::AllocationLease;
use std::sync::Arc;

#[derive(Clone)]
pub struct CoordinatorSessions {
    binding: JournalCatalogBinding,
    broker: Arc<dyn RunSessionBroker>,
    metadata: Arc<dyn PublicationWorkBudget>,
    cleanup: Arc<dyn RunSessionFactCleanup>,
}
impl CoordinatorSessions {
    /// The metadata budget is a separate finite actual owner; keeping a Live
    /// receipt must not retain command-ingress or urgent/cleanup work capacity.
    pub fn new(
        binding: JournalCatalogBinding,
        broker: Arc<dyn RunSessionBroker>,
        metadata: Arc<dyn PublicationWorkBudget>,
        cleanup: Arc<dyn RunSessionFactCleanup>,
    ) -> Result<Self, CoordinatorError> {
        if cleanup.binding() != &binding {
            return Err(CoordinatorError::StaleSource);
        }
        Ok(Self {
            binding,
            broker,
            metadata,
            cleanup,
        })
    }
    pub(super) fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub(super) fn broker(&self) -> &Arc<dyn RunSessionBroker> {
        &self.broker
    }
    pub(super) fn pending_metadata(&self) -> Result<AllocationLease, CoordinatorError> {
        self.metadata
            .try_reserve(PublicationAllocation::new(session_descriptor_bytes(), 1)?)
            .map_err(CoordinatorError::from)
    }
    pub(super) fn request_metadata(&self) -> Result<AllocationLease, CoordinatorError> {
        // Bounded SSH consent includes one public key (2KiB) in addition to
        // full source/target coordinates. Never retain urgent ingress to wait.
        self.metadata
            .try_reserve(PublicationAllocation::new(
                session_descriptor_bytes() + 4096,
                1,
            )?)
            .map_err(CoordinatorError::from)
    }
    pub(super) fn publication(&self) -> SessionPublicationResources {
        SessionPublicationResources {
            metadata: self.metadata.clone(),
            cleanup: self.cleanup.clone(),
        }
    }
}

/// No broker reference: an original broker task may retain its fact sink without
/// a broker→sink→broker strong cycle.
#[derive(Clone)]
pub(super) struct SessionPublicationResources {
    metadata: Arc<dyn PublicationWorkBudget>,
    cleanup: Arc<dyn RunSessionFactCleanup>,
}
impl SessionPublicationResources {
    pub(super) fn fact_metadata(
        &self,
        anchored: bool,
    ) -> Result<AllocationLease, CoordinatorError> {
        // Descriptive fact and immutable establishment anchor are two bounded
        // resident generations. Both share this exact two-slot reservation.
        let generations = 1 + usize::from(anchored);
        self.metadata
            .try_reserve(PublicationAllocation::new(
                generations * session_descriptor_bytes(),
                generations,
            )?)
            .map_err(CoordinatorError::from)
    }
    pub(super) fn cleanup(&self) -> &Arc<dyn RunSessionFactCleanup> {
        &self.cleanup
    }
}
