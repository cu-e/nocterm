//! Bounded account round-robin reservations; active work is never silently evicted.
use crate::{
    authority::{RunBinding, TargetBinding},
    identity::{AccountScope, SchedulerId, Timestamp},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
mod pending;
use pending::PendingSlots;
pub use pending::QueueReservation;

const MAX_GLOBAL_ACTIVE: usize = 1024;
const MAX_GLOBAL_QUEUED: usize = 4096;
const MAX_ACCOUNTS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionLimits {
    active_global: usize,
    active_account: usize,
    queued_global: usize,
    queued_account: usize,
    queued_chat: usize,
    accounts: usize,
}
impl AdmissionLimits {
    pub fn active_global(self) -> usize {
        self.active_global
    }
    pub fn new(
        active_global: usize,
        active_account: usize,
        queued_global: usize,
        queued_account: usize,
        queued_chat: usize,
        accounts: usize,
    ) -> Result<Self, AdmissionError> {
        if active_global == 0
            || active_global > MAX_GLOBAL_ACTIVE
            || active_account == 0
            || active_account > active_global
            || queued_global == 0
            || queued_global > MAX_GLOBAL_QUEUED
            || queued_account == 0
            || queued_account > queued_global
            || queued_chat == 0
            || queued_chat > queued_account
            || accounts == 0
            || accounts > MAX_ACCOUNTS
        {
            return Err(AdmissionError::InvalidLimits);
        }
        Ok(Self {
            active_global,
            active_account,
            queued_global,
            queued_account,
            queued_chat,
            accounts,
        })
    }
}
impl Default for AdmissionLimits {
    fn default() -> Self {
        Self {
            active_global: 8,
            active_account: 2,
            queued_global: 128,
            queued_account: 32,
            queued_chat: 8,
            accounts: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunAccess {
    ReadOnly,
    Mutating,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedRun {
    binding: RunBinding,
    lane: TargetBinding,
    access: RunAccess,
    expires: Timestamp,
}
impl QueuedRun {
    pub fn new(
        binding: RunBinding,
        lane: TargetBinding,
        access: RunAccess,
        expires: Timestamp,
    ) -> Self {
        Self {
            binding,
            lane,
            access,
            expires,
        }
    }
    pub fn binding(&self) -> &RunBinding {
        &self.binding
    }
    pub fn lane(&self) -> &TargetBinding {
        &self.lane
    }
    pub fn access(&self) -> RunAccess {
        self.access
    }
    pub fn expires(&self) -> Timestamp {
        self.expires
    }
}

/// A reservation belongs to one scheduler lifetime and monotonically numbered slot.
/// Returning an old reservation can never release its replacement.
#[derive(Clone, Debug)]
pub struct AdmissionLease {
    owner: SchedulerId,
    token: u64,
    request: QueuedRun,
}
impl AdmissionLease {
    pub fn request(&self) -> &QueuedRun {
        &self.request
    }
}

#[derive(Clone, Debug)]
pub struct AdmissionDecision {
    pub lease: Option<AdmissionLease>,
    pub expired: Vec<QueuedRun>,
}

#[derive(Debug)]
pub struct FairAdmission {
    owner: SchedulerId,
    queues: BTreeMap<AccountScope, VecDeque<QueuedRun>>,
    rotation: VecDeque<AccountScope>,
    active: BTreeMap<RunBinding, AdmissionLease>,
    limits: AdmissionLimits,
    next_lease: u64,
    observed: Option<Timestamp>,
    clock_failed: bool,
    pending: PendingSlots,
}
impl FairAdmission {
    pub fn new(owner: SchedulerId, limits: AdmissionLimits) -> Self {
        Self {
            owner,
            queues: BTreeMap::new(),
            rotation: VecDeque::new(),
            active: BTreeMap::new(),
            limits,
            next_lease: 0,
            observed: None,
            clock_failed: false,
            pending: PendingSlots::default(),
        }
    }
    pub fn queued_count(&self) -> usize {
        self.queues.values().map(VecDeque::len).sum()
    }
    pub fn active_count(&self) -> usize {
        self.active.len()
    }
    pub fn resident_accounts(&self) -> usize {
        self.queues
            .keys()
            .chain(self.active.keys().map(RunBinding::scope))
            .chain(self.pending.iter().map(|run| run.binding().scope()))
            .collect::<BTreeSet<_>>()
            .len()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Holds bounded capacity without creating runnable work. The host activates
    /// it only after CommandAccepted is durably acknowledged.
    pub fn prepare(
        &mut self,
        request: QueuedRun,
        now: Timestamp,
    ) -> Result<QueueReservation, AdmissionError> {
        self.observe(now)?;
        if request.expires <= now {
            return Err(AdmissionError::Expired);
        }
        if let Some(existing) = self.existing(&request.binding) {
            return Err(if existing == &request {
                AdmissionError::Duplicate
            } else {
                AdmissionError::Conflict
            });
        }
        self.check_capacity(&request)?;
        self.pending.insert(self.owner, request)
    }

    pub fn activate(
        &mut self,
        reservation: &QueueReservation,
        now: Timestamp,
    ) -> Result<(), AdmissionError> {
        self.observe(now)?;
        let request = self.pending.take(self.owner, reservation)?;
        if request.expires <= now {
            return Err(AdmissionError::Expired);
        }
        let scope = request.binding.scope().clone();
        if !self.queues.contains_key(&scope) {
            self.rotation.push_back(scope.clone());
        }
        self.queues.entry(scope).or_default().push_back(request);
        Ok(())
    }
    pub fn abandon(&mut self, reservation: &QueueReservation) -> Result<(), AdmissionError> {
        self.pending.take(self.owner, reservation).map(drop)
    }
    pub fn enqueue(&mut self, request: QueuedRun, now: Timestamp) -> Result<(), AdmissionError> {
        let reservation = self.prepare(request, now)?;
        self.activate(&reservation, now)
    }

    /// Returns every expired queued request so the host can persist visible outcomes.
    /// Active reservations remain occupied until explicit worker cleanup/release.
    pub fn expire_queued(&mut self, now: Timestamp) -> Result<Vec<QueuedRun>, AdmissionError> {
        self.observe(now)?;
        Ok(self.expire(now))
    }

    pub fn reserve(&mut self, now: Timestamp) -> Result<AdmissionDecision, AdmissionError> {
        self.observe(now)?;
        let token = self
            .next_lease
            .checked_add(1)
            .ok_or(AdmissionError::Exhausted)?;
        let expired = self.expire(now);
        let mut decision = AdmissionDecision {
            lease: None,
            expired,
        };
        if self.active.len() >= self.limits.active_global {
            return Ok(decision);
        }
        for _ in 0..self.rotation.len() {
            let Some(scope) = self.rotation.pop_front() else {
                break;
            };
            let eligible = self
                .queues
                .get(&scope)
                .and_then(|queue| queue.iter().position(|request| self.eligible(request)));
            let request = eligible.and_then(|index| self.queues.get_mut(&scope)?.remove(index));
            if self
                .queues
                .get(&scope)
                .is_some_and(|queue| !queue.is_empty())
            {
                self.rotation.push_back(scope);
            } else {
                self.queues.remove(&scope);
            }
            if let Some(request) = request {
                let lease = AdmissionLease {
                    owner: self.owner,
                    token,
                    request,
                };
                self.active
                    .insert(lease.request.binding.clone(), lease.clone());
                self.next_lease = token;
                decision.lease = Some(lease);
                break;
            }
        }
        Ok(decision)
    }

    pub fn release(&mut self, lease: &AdmissionLease) -> Result<(), AdmissionError> {
        let original = self
            .active
            .get(lease.request.binding())
            .ok_or(AdmissionError::StaleLease)?;
        if lease.owner != self.owner || original.token != lease.token {
            return Err(AdmissionError::StaleLease);
        }
        self.active.remove(lease.request.binding());
        Ok(())
    }

    /// Stop/revoke control does not need a free slot in the ordinary admission queue.
    pub fn cancel_queued(&mut self, binding: &RunBinding) -> Option<QueuedRun> {
        let request = self.queues.get_mut(binding.scope()).and_then(|queue| {
            let index = queue
                .iter()
                .position(|request| &request.binding == binding)?;
            queue.remove(index)
        });
        let request = request.or_else(|| self.pending.cancel(binding));
        self.prune();
        request
    }

    /// Expiry requests cancellation, not capacity release; remote cleanup is observable.
    pub fn expired_active(&self, now: Timestamp) -> Vec<AdmissionLease> {
        self.active
            .values()
            .filter(|lease| lease.request.expires <= now)
            .cloned()
            .collect()
    }

    fn observe(&mut self, now: Timestamp) -> Result<(), AdmissionError> {
        if self.clock_failed || self.observed.is_some_and(|previous| now < previous) {
            self.clock_failed = true;
            return Err(AdmissionError::ClockRollback);
        }
        self.observed = Some(now);
        Ok(())
    }
    fn existing(&self, binding: &RunBinding) -> Option<&QueuedRun> {
        self.active
            .get(binding)
            .map(|lease| &lease.request)
            .or_else(|| self.pending.get(binding))
            .or_else(|| {
                self.queues
                    .get(binding.scope())?
                    .iter()
                    .find(|request| &request.binding == binding)
            })
    }
    fn check_capacity(&self, request: &QueuedRun) -> Result<(), AdmissionError> {
        let queue = self.queues.get(request.binding.scope());
        let pending = self
            .pending
            .iter()
            .filter(|run| run.binding.scope() == request.binding.scope())
            .collect::<Vec<_>>();
        let known = queue.is_some()
            || !pending.is_empty()
            || self
                .active
                .keys()
                .any(|run| run.scope() == request.binding.scope());
        if !known && self.resident_accounts() >= self.limits.accounts {
            return Err(AdmissionError::AccountCapacity);
        }
        if self.queued_count().saturating_add(self.pending.len()) >= self.limits.queued_global {
            return Err(AdmissionError::GlobalCapacity);
        }
        if queue.map_or(0, VecDeque::len).saturating_add(pending.len())
            >= self.limits.queued_account
        {
            return Err(AdmissionError::AccountCapacity);
        }
        let chat_queued = queue.map_or(0, |queue| {
            queue
                .iter()
                .filter(|run| run.binding.same_chat(&request.binding))
                .count()
        });
        let chat_pending = pending
            .iter()
            .filter(|run| run.binding.same_chat(&request.binding))
            .count();
        if chat_queued.saturating_add(chat_pending) >= self.limits.queued_chat {
            return Err(AdmissionError::ChatCapacity);
        }
        Ok(())
    }
    fn eligible(&self, request: &QueuedRun) -> bool {
        let account_active = self
            .active
            .keys()
            .filter(|run| run.scope() == request.binding.scope())
            .count();
        account_active < self.limits.active_account
            && !self
                .active
                .keys()
                .any(|run| run.same_chat(&request.binding))
            && !self.active.values().any(|lease| {
                request.access == RunAccess::Mutating
                    && lease.request.access == RunAccess::Mutating
                    && lease.request.binding.scope() == request.binding.scope()
                    && lease.request.lane.same_mutation_lane(&request.lane)
            })
    }
    fn expire(&mut self, now: Timestamp) -> Vec<QueuedRun> {
        let mut expired = Vec::new();
        for queue in self.queues.values_mut() {
            let mut kept = VecDeque::new();
            while let Some(request) = queue.pop_front() {
                if request.expires <= now {
                    expired.push(request);
                } else {
                    kept.push_back(request);
                }
            }
            *queue = kept;
        }
        self.prune();
        expired
    }
    fn prune(&mut self) {
        self.queues.retain(|_, queue| !queue.is_empty());
        let queues = &self.queues;
        self.rotation.retain(|scope| queues.contains_key(scope));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("admission limits must be positive and bounded")]
    InvalidLimits,
    #[error("the queued run expired before admission")]
    Expired,
    #[error("this run is already queued or active")]
    Duplicate,
    #[error("the run identity was reused with different admission data")]
    Conflict,
    #[error("the global admission queue is full")]
    GlobalCapacity,
    #[error("the account admission capacity is full")]
    AccountCapacity,
    #[error("the chat admission queue is full")]
    ChatCapacity,
    #[error("the original scheduler reservation no longer matches")]
    StaleLease,
    #[error("the admission reservation counter is exhausted")]
    Exhausted,
    #[error("the clock moved backwards; new admissions are fenced")]
    ClockRollback,
}
