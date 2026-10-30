//! Explicit memory-only provider access with a finite monotonic deadline.
use crate::config::{ProviderBinding, ProviderKind};
use futures::future::{AbortHandle, AbortRegistration};
use nocterm_cloud_model::coordinator::ProviderCredentialSource;
use nocterm_cloud_model::engine::EngineError;
use nocterm_cloud_model::{identity::Timestamp, tool::ToolTimer};
use nocterm_core::allocation::AllocationLease;
use rig_core::http_client::HeaderValue;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    time::Duration,
};
use tokio::time::Instant;
use zeroize::Zeroizing;

mod renewal;
mod services;
pub use renewal::{
    DevicePresence, LeaseRenewal, RenewalError, RenewalPolicy, RenewalPolicyError, RenewalSetup,
};
pub(crate) use services::ObservedAdmission;
pub use services::{LeaseServices, ModelUsageObserver};

const MAX_ACTIVE_REQUESTS: usize = 32;
pub const MAX_PROVIDER_LEASE: Duration = Duration::from_secs(30 * 60);

/// The host retains a clone to revoke access. Clones share one zeroizing key;
/// none exports it. New worker boots require a newly installed lease.
#[derive(Clone)]
pub struct ProviderLease {
    owner: Arc<LeaseOwner>,
}
impl ProviderLease {
    /// Requires the headless host's current Tokio runtime for expiry cleanup.
    pub fn new(
        binding: ProviderBinding,
        key: Zeroizing<String>,
        lifetime: Duration,
    ) -> Result<Self, EngineError> {
        Self::create(binding, key, lifetime, None)
    }
    /// The original capsule reservation also owns the copied key through all
    /// factories and admitted requests. Source fencing runs before every send.
    pub fn with_residency(
        binding: ProviderBinding,
        key: Zeroizing<String>,
        lifetime: Duration,
        allocation: AllocationLease,
        source: Arc<dyn ProviderCredentialSource>,
        timer: Arc<dyn ToolTimer>,
        expires: Timestamp,
    ) -> Result<Self, EngineError> {
        if allocation.bytes() < key.len() {
            return Err(EngineError::Unavailable);
        }
        Self::create(
            binding,
            key,
            lifetime,
            Some(LeaseResidency {
                source,
                timer,
                expires: Mutex::new(expires),
                observed: Mutex::new(None),
                _allocation: allocation,
            }),
        )
    }
    fn create(
        binding: ProviderBinding,
        key: Zeroizing<String>,
        lifetime: Duration,
        residency: Option<LeaseResidency>,
    ) -> Result<Self, EngineError> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| EngineError::Unavailable)?;
        if lifetime.is_zero()
            || lifetime > MAX_PROVIDER_LEASE
            || key.is_empty()
            || key.len() > 16 * 1024
            || key.bytes().any(|byte| !byte.is_ascii_graphic())
        {
            return Err(EngineError::NeedsProviderAccess);
        }
        let owner = Arc::new(LeaseOwner {
            binding,
            state: Mutex::new(LeaseState {
                key: Some(key),
                next: 0,
                requests: BTreeMap::new(),
            }),
            deadline: Mutex::new(Instant::now() + lifetime),
            expiry: Mutex::new(None),
            residency,
            renewal: Mutex::new(None),
            usage: OnceLock::new(),
        });
        let revocation = RevokeOnDrop(Arc::downgrade(&owner));
        let weak = Arc::downgrade(&owner);
        let task = runtime.spawn(async move {
            let _revocation = revocation;
            // The deadline moves when a renewal extends it, so re-read it.
            loop {
                let Some(owner) = weak.upgrade() else { return };
                let deadline = owner.deadline();
                drop(owner);
                tokio::time::sleep_until(deadline).await;
                let Some(owner) = weak.upgrade() else { return };
                if owner.deadline() <= Instant::now() {
                    owner.revoke();
                    return;
                }
            }
        });
        *owner
            .expiry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Expiry(task.abort_handle()));
        Ok(Self { owner })
    }
    pub fn binding(&self) -> &ProviderBinding {
        &self.owner.binding
    }
    pub(crate) fn allows_run(&self, run: &nocterm_cloud_model::authority::RunBinding) -> bool {
        self.owner.residency.as_ref().is_none_or(|residency| {
            let binding = residency.source.binding();
            run.scope() == binding.scope()
                && run.runner() == binding.runner()
                && run.runner_epoch() == binding.epoch()
        })
    }
    pub fn revoke(&self) {
        self.owner.revoke();
    }
    /// Pushes the deadline to one renewal step from now, never beyond
    /// [`MAX_PROVIDER_LEASE`] ahead. A revoked or expired lease cannot be renewed.
    pub(crate) fn extend(&self, step: Duration) -> Result<(), EngineError> {
        if step.is_zero() || step > MAX_PROVIDER_LEASE || !self.is_available() {
            return Err(EngineError::NeedsProviderAccess);
        }
        if let Some(residency) = &self.owner.residency {
            residency.extend(step);
        }
        let mut deadline = self
            .owner
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *deadline = (*deadline).max(Instant::now() + step);
        Ok(())
    }
    pub(crate) fn remaining(&self) -> Option<Duration> {
        self.is_available().then(|| {
            self.owner
                .deadline()
                .saturating_duration_since(Instant::now())
        })
    }
    /// The furthest wall-clock instant the original host grant currently allows.
    pub(crate) fn horizon(&self) -> Option<Timestamp> {
        self.owner.residency.as_ref().map(LeaseResidency::expires)
    }
    /// Keeps the lease alive in the background while the presence port reports
    /// a device online. Replaces a previous renewal; stops with the lease.
    pub(crate) fn attach(&self, services: &LeaseServices) {
        if let Some(usage) = services.usage() {
            let _ = self.owner.usage.set(usage.clone());
        }
        if let Some(setup) = services.renewal()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let renewal = LeaseRenewal::start(&runtime, Arc::downgrade(&self.owner), setup);
            *self
                .owner
                .renewal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(renewal);
        }
    }
    /// Ends background renewal; the lease still lapses at its current deadline.
    pub fn stop_renewal(&self) {
        self.owner.stop_renewal();
    }
    pub(crate) fn usage(&self) -> Option<Arc<dyn ModelUsageObserver>> {
        self.owner.usage.get().cloned()
    }
    pub fn is_available(&self) -> bool {
        if Instant::now() >= self.owner.deadline()
            || self
                .owner
                .residency
                .as_ref()
                .is_some_and(|guard| !guard.current())
        {
            self.owner.revoke();
        }
        self.owner.state().key.is_some()
    }
    pub(crate) fn admit(&self) -> Result<(LeaseOperation, AbortRegistration), EngineError> {
        if !self.is_available() {
            return Err(EngineError::NeedsProviderAccess);
        }
        let mut state = self.owner.state();
        if state.key.is_none() {
            return Err(EngineError::NeedsProviderAccess);
        }
        if state.requests.len() >= MAX_ACTIVE_REQUESTS {
            return Err(EngineError::BudgetExceeded);
        }
        let id = state.next;
        state.next = id.checked_add(1).ok_or(EngineError::Unavailable)?;
        let (handle, registration) = AbortHandle::new_pair();
        state.requests.insert(id, handle);
        Ok((
            LeaseOperation {
                owner: self.owner.clone(),
                id,
            },
            registration,
        ))
    }
    pub(crate) fn header(&self, kind: ProviderKind) -> Result<HeaderValue, EngineError> {
        if !self.is_available() {
            return Err(EngineError::NeedsProviderAccess);
        }
        if kind != self.owner.binding.kind() {
            return Err(EngineError::Unavailable);
        }
        let state = self.owner.state();
        let key = state.key.as_ref().ok_or(EngineError::NeedsProviderAccess)?;
        let header = match kind {
            ProviderKind::OpenAiChat => Zeroizing::new(format!("Bearer {}", key.as_str())),
            ProviderKind::AnthropicMessages => Zeroizing::new(key.to_string()),
        };
        let mut value =
            HeaderValue::from_str(&header).map_err(|_| EngineError::NeedsProviderAccess)?;
        value.set_sensitive(true);
        Ok(value)
    }
}
impl std::fmt::Debug for ProviderLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderLease([redacted])")
    }
}
struct LeaseState {
    key: Option<Zeroizing<String>>,
    next: u64,
    requests: BTreeMap<u64, AbortHandle>,
}
struct LeaseOwner {
    binding: ProviderBinding,
    state: Mutex<LeaseState>,
    deadline: Mutex<Instant>,
    expiry: Mutex<Option<Expiry>>,
    residency: Option<LeaseResidency>,
    renewal: Mutex<Option<LeaseRenewal>>,
    usage: OnceLock<Arc<dyn ModelUsageObserver>>,
}
struct LeaseResidency {
    source: Arc<dyn ProviderCredentialSource>,
    timer: Arc<dyn ToolTimer>,
    expires: Mutex<Timestamp>,
    observed: Mutex<Option<Timestamp>>,
    _allocation: AllocationLease,
}
impl LeaseResidency {
    fn expires(&self) -> Timestamp {
        *self
            .expires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn extend(&self, step: Duration) {
        let step = u64::try_from(step.as_millis()).unwrap_or(u64::MAX);
        let target = Timestamp::from_millis(self.timer.now().as_millis().saturating_add(step));
        let mut expires = self
            .expires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *expires = (*expires).max(target);
    }
    fn current(&self) -> bool {
        if self.source.check_current().is_err() {
            return false;
        }
        let now = self.timer.now();
        let Ok(mut observed) = self.observed.lock() else {
            return false;
        };
        if now >= self.expires() || observed.is_some_and(|previous| now < previous) {
            return false;
        }
        *observed = Some(now);
        true
    }
}
impl LeaseOwner {
    fn deadline(&self) -> Instant {
        *self
            .deadline
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    fn stop_renewal(&self) {
        self.renewal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
    fn state(&self) -> MutexGuard<'_, LeaseState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poison) => {
                let mut state = poison.into_inner();
                // A poisoned owner fails closed and still zeroizes its key.
                state.key.take();
                for request in state.requests.values() {
                    request.abort();
                }
                state
            }
        }
    }
    fn revoke(&self) {
        self.stop_renewal();
        let mut state = self.state();
        state.key.take();
        for request in state.requests.values() {
            request.abort();
        }
        state.requests.clear();
    }
}
struct Expiry(tokio::task::AbortHandle);
impl Drop for Expiry {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub(crate) struct LeaseOperation {
    owner: Arc<LeaseOwner>,
    id: u64,
}
impl Drop for LeaseOperation {
    fn drop(&mut self) {
        self.owner.state().requests.remove(&self.id);
    }
}

// Cancellation or destruction of the host runtime also revokes the lease,
// including when this timer task was never polled.
struct RevokeOnDrop(std::sync::Weak<LeaseOwner>);
impl Drop for RevokeOnDrop {
    fn drop(&mut self) {
        if let Some(owner) = self.0.upgrade() {
            owner.revoke();
        }
    }
}
