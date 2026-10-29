//! Shared original-account capture for native vault, replica and credential workers.
use super::{
    OpenKeys, Slot, SlotReceipt,
    source::{CapturedAccount, CapturedMembership, LocalSourceProvider},
};
use crate::{
    pool::Pool,
    store::{HubStore, StoreOps},
    vault::Ledger,
};
use nocterm_hubs_model::{
    HubError,
    source_identity::{LocalAccountPartition, LocalAccountSource, LocalAccountSourceError},
};
use std::sync::{Arc, Weak};

/// The single shared store selector. Production uses the original SDK client;
/// MemoryHub implements this seam in the existing native test-support owner.
pub(crate) trait OperationStores: Send + Sync {
    fn opened(
        &self,
        native: &CapturedAccount,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<dyn StoreOps>, HubError>;
    fn membership(&self, native: &CapturedMembership) -> Result<Arc<dyn StoreOps>, HubError>;
}

struct PooledStores(Weak<Pool>);
impl OperationStores for PooledStores {
    fn opened(
        &self,
        native: &CapturedAccount,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<dyn StoreOps>, HubError> {
        let pool = self.0.upgrade().ok_or(HubError::Stale)?;
        Ok(Arc::new(HubStore::captured(pool, native, source)?))
    }
    fn membership(&self, native: &CapturedMembership) -> Result<Arc<dyn StoreOps>, HubError> {
        let pool = self.0.upgrade().ok_or(HubError::Stale)?;
        Ok(Arc::new(HubStore::membership(pool, native)?))
    }
}

/// A caller-provided lease only restricts the independently authenticated capture.
pub(crate) struct OpenedSource {
    native: Arc<CapturedAccount>,
    restriction: Arc<dyn LocalAccountSource>,
}
impl OpenedSource {
    pub(crate) fn new(
        native: Arc<CapturedAccount>,
        restriction: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<Self>, HubError> {
        if native.partition() != restriction.partition() {
            return Err(HubError::Stale);
        }
        let source = Arc::new(Self {
            native,
            restriction,
        });
        source.check_current().map_err(|_| HubError::Stale)?;
        Ok(source)
    }
    pub(crate) fn native(&self) -> &Arc<CapturedAccount> {
        &self.native
    }
    pub(crate) fn check(&self) -> Result<(), HubError> {
        self.check_current().map_err(|_| HubError::Stale)
    }
    pub(crate) fn publish<T>(&self, publish: impl FnOnce() -> T) -> Result<T, HubError> {
        self.restriction
            .check_current()
            .map_err(|_| HubError::Stale)?;
        self.native.publish(publish)
    }
    pub(crate) async fn during<T>(
        &self,
        future: impl std::future::Future<Output = T>,
    ) -> Result<T, HubError> {
        let mut future = std::pin::pin!(future);
        std::future::poll_fn(|cx| {
            if let Err(error) = self.check() {
                return std::task::Poll::Ready(Err(error));
            }
            match future.as_mut().poll(cx) {
                std::task::Poll::Pending => std::task::Poll::Pending,
                std::task::Poll::Ready(value) => {
                    std::task::Poll::Ready(self.check().map(|()| value))
                }
            }
        })
        .await
    }
}
impl LocalAccountSource for OpenedSource {
    fn partition(&self) -> LocalAccountPartition {
        self.native.partition()
    }
    fn check_current(&self) -> Result<(), LocalAccountSourceError> {
        self.native.check_current()?;
        self.restriction.check_current()
    }
}

/// A bound facade retains weak key authority; only admitted workers hold keys.
#[derive(Clone)]
pub(crate) struct BoundOperations {
    source: Arc<OpenedSource>,
    stores: Arc<dyn OperationStores>,
}
impl BoundOperations {
    pub(crate) fn source(&self) -> &Arc<OpenedSource> {
        &self.source
    }
    pub(crate) fn capture(&self) -> Result<OpenedWork, HubError> {
        self.source.check()?;
        let keys = self.source.native.keys()?;
        let ops = self
            .stores
            .opened(&self.source.native, self.source.clone())?;
        self.source.check()?;
        Ok(OpenedWork {
            source: self.source.clone(),
            keys,
            ops,
        })
    }
}

pub(crate) struct OpenedWork {
    source: Arc<OpenedSource>,
    keys: Arc<OpenKeys>,
    ops: Arc<dyn StoreOps>,
}
impl OpenedWork {
    pub(crate) fn source(&self) -> &Arc<OpenedSource> {
        &self.source
    }
    pub(crate) fn store(&self) -> crate::store::Store {
        crate::store::Store::new(self.ops.clone(), self.keys.clone())
    }
    pub(crate) fn ledger(&self) -> Result<Ledger, HubError> {
        self.source.check()?;
        Ok(Ledger::captured(
            self.source.native.slot()?,
            self.ops.clone(),
            self.source.native.clone(),
        ))
    }
    pub(crate) fn ops(&self) -> &Arc<dyn StoreOps> {
        &self.ops
    }
}

pub(crate) struct MembershipWork {
    membership: Arc<CapturedMembership>,
    receipt: SlotReceipt,
    ops: Arc<dyn StoreOps>,
}
impl MembershipWork {
    pub(crate) fn membership(&self) -> &Arc<CapturedMembership> {
        &self.membership
    }
    pub(crate) fn receipt(&self) -> &SlotReceipt {
        &self.receipt
    }
    pub(crate) fn ops(&self) -> &Arc<dyn StoreOps> {
        &self.ops
    }
    pub(crate) fn slot(&self) -> Result<Arc<Slot>, HubError> {
        self.membership.slot()
    }
    pub(crate) fn check(&self) -> Result<(), HubError> {
        self.membership.with_current(|| {
            self.slot()?
                .check_receipt(&self.membership.subject()?, &self.receipt)
        })
    }
}

pub(crate) struct AccountOperations {
    provider: LocalSourceProvider,
    stores: Arc<dyn OperationStores>,
}
impl AccountOperations {
    pub(crate) fn current_opened_source(&self) -> Result<Arc<CapturedAccount>, HubError> {
        self.provider.capture_native()
    }
    pub(crate) fn current_membership(&self) -> Result<Arc<CapturedMembership>, HubError> {
        self.provider.membership()
    }
    #[cfg(test)]
    pub(crate) fn with_stores(
        provider: LocalSourceProvider,
        stores: Arc<dyn OperationStores>,
    ) -> Self {
        Self { provider, stores }
    }
    pub(crate) fn new(provider: LocalSourceProvider, pool: &Arc<Pool>) -> Self {
        Self {
            provider,
            stores: Arc::new(PooledStores(Arc::downgrade(pool))),
        }
    }
    pub(crate) fn stores(&self) -> &Arc<dyn OperationStores> {
        &self.stores
    }
    pub(crate) fn bind(
        &self,
        restriction: Arc<dyn LocalAccountSource>,
    ) -> Result<BoundOperations, HubError> {
        let source = OpenedSource::new(self.provider.capture_native()?, restriction)?;
        Ok(BoundOperations {
            source,
            stores: self.stores.clone(),
        })
    }
    pub(crate) fn opened(&self) -> Result<OpenedWork, HubError> {
        let native = self.provider.capture_native()?;
        let source = OpenedSource::new(native.clone(), native)?;
        BoundOperations {
            source,
            stores: self.stores.clone(),
        }
        .capture()
    }
    pub(crate) fn membership(&self) -> Result<MembershipWork, HubError> {
        let membership = self.provider.membership()?;
        let receipt =
            membership.with_current(|| membership.slot()?.receipt(&membership.subject()?))?;
        let ops = self.stores.membership(&membership)?;
        let work = MembershipWork {
            membership,
            receipt,
            ops,
        };
        work.check()?;
        Ok(work)
    }
}
