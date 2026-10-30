//! Finite injected global/account/chat admission and actual last-drop counters.
use nocterm_cloud_model::{
    coordinator::CoordinatorError,
    journal::{PublicationAllocation, PublicationWorkBudget},
};
use nocterm_core::{allocation::AllocationLease, persist::ShutdownDeadline};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

pub(crate) struct SessionCapacity {
    handshakes: Arc<Semaphore>,
    physical: Arc<Semaphore>,
    metadata: Arc<dyn PublicationWorkBudget>,
}
impl SessionCapacity {
    pub(crate) fn new(
        handshakes: usize,
        physical: usize,
        metadata: Arc<dyn PublicationWorkBudget>,
    ) -> Result<Arc<Self>, CoordinatorError> {
        if handshakes == 0 || handshakes > 64 || physical == 0 || physical > 256 {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Arc::new(Self {
            handshakes: Arc::new(Semaphore::new(handshakes)),
            physical: Arc::new(Semaphore::new(physical)),
            metadata,
        }))
    }
}
pub(crate) struct ChatCapacity {
    handshake: Arc<Semaphore>,
    physical: Arc<Semaphore>,
}
impl ChatCapacity {
    pub(crate) fn new() -> Self {
        Self {
            handshake: Arc::new(Semaphore::new(1)),
            physical: Arc::new(Semaphore::new(16)),
        }
    }
}
#[derive(Clone)]
pub(crate) struct SessionResources {
    global: Arc<SessionCapacity>,
    account: Arc<SessionCapacity>,
    counts: Arc<Counts>,
}
#[derive(Default)]
struct Counts {
    physical: AtomicUsize,
    handshakes: AtomicUsize,
    changed: Notify,
}
pub(crate) struct SessionAdmission {
    residency: Arc<PhysicalResidency>,
    handshake: Handshake,
}
impl SessionAdmission {
    pub(crate) fn residency(&self) -> Arc<dyn Send + Sync> {
        self.residency.clone()
    }
    pub(crate) fn allocation(&self) -> &AllocationLease {
        &self.residency.allocation
    }
    pub(crate) fn established(self) -> Arc<dyn Send + Sync> {
        let Self {
            residency,
            handshake,
        } = self;
        drop(handshake);
        residency
    }
}
struct PhysicalResidency {
    _global: OwnedSemaphorePermit,
    _account: OwnedSemaphorePermit,
    _chat: OwnedSemaphorePermit,
    allocation: AllocationLease,
    _count: Counted,
}
struct Handshake {
    _global: OwnedSemaphorePermit,
    _account: OwnedSemaphorePermit,
    _chat: OwnedSemaphorePermit,
    _count: Counted,
}
enum Kind {
    Physical,
    Handshake,
}
struct Counted {
    owner: Arc<Counts>,
    kind: Kind,
}
impl Counted {
    fn new(owner: Arc<Counts>, kind: Kind) -> Self {
        match kind {
            Kind::Physical => owner.physical.fetch_add(1, Ordering::AcqRel),
            Kind::Handshake => owner.handshakes.fetch_add(1, Ordering::AcqRel),
        };
        Self { owner, kind }
    }
}
impl Drop for Counted {
    fn drop(&mut self) {
        match self.kind {
            Kind::Physical => self.owner.physical.fetch_sub(1, Ordering::AcqRel),
            Kind::Handshake => self.owner.handshakes.fetch_sub(1, Ordering::AcqRel),
        };
        self.owner.changed.notify_waiters();
    }
}
impl SessionResources {
    /// Reuse the same global owner across principals. The account owner remains
    /// specific to the configured isolated principal; neither is a static/global.
    pub(crate) fn new(global: Arc<SessionCapacity>, account: Arc<SessionCapacity>) -> Self {
        Self {
            global,
            account,
            counts: Arc::new(Counts::default()),
        }
    }
    pub(crate) fn reserve(
        &self,
        chat: &ChatCapacity,
    ) -> Result<SessionAdmission, CoordinatorError> {
        let global = take(&self.global.physical)?;
        let account = take(&self.account.physical)?;
        let local = take(&chat.physical)?;
        let global_work = self
            .global
            .metadata
            .try_reserve(PublicationAllocation::new(
                super::targets::TARGET_CAPTURE_BYTES,
                1,
            )?)?;
        let account_work = self
            .account
            .metadata
            .try_reserve(PublicationAllocation::new(
                super::targets::TARGET_CAPTURE_BYTES,
                1,
            )?)?;
        let allocation = AllocationLease::new(
            super::targets::TARGET_CAPTURE_BYTES,
            1,
            (global_work, account_work),
        );
        let handshake = Handshake {
            _global: take(&self.global.handshakes)?,
            _account: take(&self.account.handshakes)?,
            _chat: take(&chat.handshake)?,
            _count: Counted::new(self.counts.clone(), Kind::Handshake),
        };
        Ok(SessionAdmission {
            residency: Arc::new(PhysicalResidency {
                _global: global,
                _account: account,
                _chat: local,
                allocation,
                _count: Counted::new(self.counts.clone(), Kind::Physical),
            }),
            handshake,
        })
    }
    pub(crate) fn pending(&self) -> (usize, usize) {
        (
            self.counts.handshakes.load(Ordering::Acquire),
            self.counts.physical.load(Ordering::Acquire),
        )
    }
    pub(crate) async fn drain(&self, deadline: ShutdownDeadline) -> bool {
        loop {
            let changed = self.counts.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.pending() == (0, 0) {
                return true;
            }
            if tokio::time::timeout(deadline.remaining(), changed)
                .await
                .is_err()
            {
                return false;
            }
        }
    }
}
fn take(semaphore: &Arc<Semaphore>) -> Result<OwnedSemaphorePermit, CoordinatorError> {
    semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| CoordinatorError::Overloaded)
}
