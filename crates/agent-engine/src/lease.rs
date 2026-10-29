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
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

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
                expires,
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
            deadline: Instant::now() + lifetime,
            expiry: Mutex::new(None),
            residency,
        });
        let revocation = RevokeOnDrop(Arc::downgrade(&owner));
        let deadline = tokio::time::Instant::from_std(owner.deadline);
        let task = runtime.spawn(async move {
            let _revocation = revocation;
            tokio::time::sleep_until(deadline).await;
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
    pub fn is_available(&self) -> bool {
        if Instant::now() >= self.owner.deadline
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
    deadline: Instant,
    expiry: Mutex<Option<Expiry>>,
    residency: Option<LeaseResidency>,
}
struct LeaseResidency {
    source: Arc<dyn ProviderCredentialSource>,
    timer: Arc<dyn ToolTimer>,
    expires: Timestamp,
    observed: Mutex<Option<Timestamp>>,
    _allocation: AllocationLease,
}
impl LeaseResidency {
    fn current(&self) -> bool {
        if self.source.check_current().is_err() {
            return false;
        }
        let now = self.timer.now();
        let Ok(mut observed) = self.observed.lock() else {
            return false;
        };
        if now >= self.expires || observed.is_some_and(|previous| now < previous) {
            return false;
        }
        *observed = Some(now);
        true
    }
}
impl LeaseOwner {
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
