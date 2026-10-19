//! The original native connection producer and prepaid physical allocation context.
//! Contexts may retain already-paid buffers after teardown; only the actual lease
//! owns the original Source capability. Its close revokes further allocation.
#![expect(
    clippy::result_large_err,
    reason = "refusals return ledger-paid values; a Box would be an unpaid allocation"
)]
use crate::model::{
    json::{
        deferred::{DecodeLedger, JsonDecodeError, NativeAllocationError, ledger::Reservation},
        lifetime::Shared,
    },
    retention::{
        RetentionError,
        document::{AllocationPlan, ConstructionRole, DocumentRetention, PendingAllocation},
    },
};
use std::{alloc::Layout, sync::Mutex};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("incorrect native source capability")]
    WrongRole,
    #[error("the original native source is still in use")]
    Unavailable,
    #[error("native source ownership is poisoned")]
    Poisoned,
    #[error(transparent)]
    Retention(#[from] RetentionError),
    #[error(transparent)]
    Allocation(#[from] JsonDecodeError),
}
struct ProducerBody {
    available: Mutex<Option<ConstructionRole>>,
    scope: DocumentRetention,
    _allocation: PendingAllocation,
}
#[derive(Clone)]
pub struct SourceProducer(Shared<ProducerBody>);
impl SourceProducer {
    pub fn same_document(&self, other: &Self) -> bool {
        self.0.scope.same_document(&other.0.scope)
    }
    pub fn new(mut original: ConstructionRole) -> Result<Self, SourceError> {
        if !original.is_source() {
            return Err(SourceError::WrongRole);
        }
        let scope = original.scope().clone();
        let allocation = scope.reserve_phase_metadata(AllocationPlan::shared::<ProducerBody>())?;
        original.close_phase();
        Ok(Self(Shared::new(ProducerBody {
            available: Mutex::new(Some(original)),
            scope,
            _allocation: allocation,
        })))
    }
    pub fn capture(&self) -> Result<SourceLease, SourceError> {
        let role = self
            .0
            .available
            .lock()
            .map_err(|_| SourceError::Poisoned)?
            .take()
            .ok_or(SourceError::Unavailable)?;
        let mut lease = SourceLease {
            role: Some(role),
            producer: self.clone(),
            context: None,
        };
        lease
            .role
            .as_mut()
            .ok_or(SourceError::Unavailable)?
            .begin_phase()?;
        lease.context = Some(SourceContext::new(
            lease.role.as_ref().ok_or(SourceError::Unavailable)?,
        )?);
        Ok(lease)
    }
}
/// Not Clone. Native connection teardown destroys its actual tasks before this.
pub struct SourceLease {
    role: Option<ConstructionRole>,
    producer: SourceProducer,
    context: Option<SourceContext>,
}
impl SourceLease {
    pub fn into_lifetime(self) -> Result<SourceLifetime, SourceError> {
        SourceLifetime::new(self)
    }
    pub fn context(&self) -> Result<SourceContext, SourceError> {
        self.context
            .as_ref()
            .cloned()
            .ok_or(SourceError::Unavailable)
    }
}
impl Drop for SourceLease {
    fn drop(&mut self) {
        let Some(mut original) = self.role.take() else {
            return;
        };
        original.close_phase();
        if let Ok(mut available) = self.producer.0.available.lock()
            && available.is_none()
        {
            *available = Some(original);
        }
    }
}
struct ContextBody {
    ledger: DecodeLedger,
    scope: DocumentRetention,
    _allocation: PendingAllocation,
}
#[derive(Clone)]
pub struct SourceContext(Shared<ContextBody>);
impl SourceContext {
    pub fn copy_path(
        &self,
        value: &std::path::Path,
    ) -> Result<crate::model::native::NativeOwned<std::path::PathBuf>, JsonDecodeError> {
        crate::model::native::NativeOwned::copy_path(self.ledger(), value)
    }
    pub fn prepare_error<E: std::error::Error + Send + Sync + 'static>(
        &self,
        nested_error_bytes: usize,
    ) -> Result<SourceErrorSlot<E>, NativeAllocationError> {
        SourceErrorSlot::prepare(self, nested_error_bytes)
    }
    pub fn prepare_startup(
        &self,
        request: crate::connection::ManagedStartupRequest<'_>,
    ) -> Result<
        crate::model::native::NativeOwned<crate::connection::ManagedStartupFacts>,
        JsonDecodeError,
    > {
        crate::connection::ManagedStartupFacts::prepare(self, request)
    }
    pub fn certify_opened(
        &self,
        input: crate::model::native::NativeOwned<crate::model::session::SessionOpened>,
    ) -> Result<ManagedOpened, SourceSemanticRefusal<crate::model::session::SessionOpened>> {
        ManagedOpened::certify(self, input)
    }
    pub fn certify_permission(
        &self,
        input: crate::model::native::NativeOwned<crate::model::auth::ProviderPermission>,
    ) -> Result<ManagedPermission, SourceSemanticRefusal<crate::model::auth::ProviderPermission>>
    {
        ManagedPermission::certify(self, input)
    }
    pub fn certify_info(
        &self,
        input: crate::model::native::NativeOwned<crate::connection::AgentInfo>,
    ) -> Result<ManagedAgentInfo, SourceSemanticRefusal<crate::connection::AgentInfo>> {
        ManagedAgentInfo::certify(self, input)
    }
    pub fn certify_notice(
        &self,
        input: crate::model::native::NativeOwned<crate::model::notice::SessionNotice>,
    ) -> Result<ManagedNotice, SourceSemanticRefusal> {
        ManagedNotice::certify(self, input)
    }
    fn new(role: &ConstructionRole) -> Result<Self, SourceError> {
        let allocation = role
            .scope()
            .reserve_phase_metadata(AllocationPlan::shared::<ContextBody>())?;
        let ledger = DecodeLedger::for_source(role, crate::history::MAX_RESIDENT_BYTES)?;
        Ok(Self(Shared::new(ContextBody {
            ledger,
            scope: role.scope().clone(),
            _allocation: allocation,
        })))
    }
    pub fn ledger(&self) -> &DecodeLedger {
        &self.0.ledger
    }
    pub fn same_source(&self, other: &Self) -> bool {
        std::ptr::eq(self.0.get(), other.0.get())
    }
    pub fn same_document(&self, other: &Self) -> bool {
        self.0.scope.same_document(&other.0.scope)
    }
    /// Approve an actual native layout before allocation; its owner must free
    /// the complete backing before dropping this non-cloneable receipt.
    pub fn reserve_native(&self, layout: Layout) -> Result<NativeAllocation, JsonDecodeError> {
        self.reserve_requested(layout).map_err(Into::into)
    }
    pub fn reserve_requested(
        &self,
        layout: Layout,
    ) -> Result<NativeAllocation, NativeAllocationError> {
        Ok(NativeAllocation {
            allocation: self.0.ledger.reserve_requested(layout.size())?,
            _source: self.clone(),
        })
    }
}
/// No payload or scope lookup; the concrete SDK buffer controls physical order.
pub struct NativeAllocation {
    allocation: Reservation,
    _source: SourceContext,
}
impl NativeAllocation {
    pub(crate) fn source(&self) -> &SourceContext {
        &self._source
    }
    pub fn requested_bytes(&self) -> usize {
        self.allocation.bytes()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod header_tests;

#[cfg(test)]
mod fork_tests;

mod future;
pub use future::ManagedFuture;
mod lifetime;
pub use lifetime::{ManagedDispatchFuture, ManagedSourceFuture, SourceLifetime};
mod failure;
pub use failure::{ManagedCommandError, SourceErrorSlot};

mod policy;
pub(in crate::history) use policy::modes_policy;
mod semantic;
pub use semantic::{
    ManagedAgentInfo, ManagedNotice, ManagedOpened, ManagedPermission, SourceSemanticError,
    SourceSemanticRefusal,
};
