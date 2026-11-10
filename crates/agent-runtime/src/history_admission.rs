//! One installation owns Local/remote history bytes and retained payload slots.
//!
//! Participants expose only history accounting and optional hidden-history eviction.
//! Registration and allocation tokens never keep an entity alive. No budget state
//! borrow is held while a participant is observed or asked to evict.
use crate::HistoryUsage;
use gpui::{App, EntityId};
use nocterm_ai::{
    execution_origin::ChatKey, history::MAX_RESIDENT_BYTES, storage::AgentStorageError,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::{Rc, Weak},
    sync::{Arc, Weak as SyncWeak},
};

mod eviction;
mod local_allocations;
pub use local_allocations::{LocalHistoryAllocations, LocalSnapshotAllocation};
pub(crate) mod local;
use eviction::EvictionCandidate;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub const MAX_HISTORY_PAYLOADS: usize = 32;
/// The final two payloads and 1MiB remain available to original urgent controls.
pub const MAX_NORMAL_HISTORY_PAYLOADS: usize = MAX_HISTORY_PAYLOADS - 2;
// Guarded fresh-head receipts and upload buffers share this reserved lane.
// This repartitions the same installation total; normal capacity decreases.
pub const URGENT_HISTORY_BYTES: usize = 1024 * 1024;
pub const MAX_NORMAL_HISTORY_BYTES: usize = MAX_RESIDENT_BYTES - URGENT_HISTORY_BYTES;

#[derive(Debug, thiserror::Error)]
pub enum HistoryAdmissionError {
    #[error("History admission is closed.")]
    Closed,
    #[error("History admission is already deciding an eviction.")]
    Reentrant,
    #[error("The history registration belongs to another installation.")]
    ForeignRegistration,
    #[error("The history allocation belongs to another installation.")]
    ForeignAllocation,
    #[error("A retained history allocation cannot grow from {reserved} to {requested} bytes.")]
    AllocationGrowth { reserved: usize, requested: usize },
    #[error("All normal installation history payload slots are retained or loading.")]
    PayloadCapacity,
    #[error(
        "History needs {incoming} bytes with {retained} retained; the installation limit is {limit}."
    )]
    ResidentCapacity {
        retained: usize,
        incoming: usize,
        limit: usize,
    },
    #[error(transparent)]
    Storage(#[from] AgentStorageError),
}
impl HistoryAdmissionError {
    pub fn into_storage(self) -> AgentStorageError {
        match self {
            Self::Storage(error) => error,
            Self::PayloadCapacity => AgentStorageError::PayloadCapacity,
            Self::ResidentCapacity {
                retained,
                incoming,
                limit,
            } => AgentStorageError::ResidentCapacity {
                retained,
                incoming,
                limit,
            },
            _ => AgentStorageError::Interrupted,
        }
    }
}

/// The implementation retains a weak entity/data-owner reference, never an App.
pub trait HistoryParticipant {
    fn entity_id(&self) -> Option<EntityId> {
        None
    }
    fn usage(&self, cx: &App) -> Result<Option<HistoryUsage>, AgentStorageError>;
    fn evict(&self, _cx: &mut App) -> bool {
        false
    }
    /// Oldest reclaimable payload's installation allocation order. Retained
    /// caller snapshots, shown documents and pending work are never candidates.
    fn oldest_payload(&self, _cx: &App) -> Option<u64> {
        None
    }
    fn evict_payload(&self, _cx: &mut App) -> bool {
        false
    }
    fn local_key(&self, _cx: &App) -> Option<ChatKey> {
        None
    }
}
struct RegisteredHistory {
    owner: Weak<InstallationHistoryAdmission>,
    participant: Rc<dyn HistoryParticipant>,
}
/// Retains the observation adapter, whose entity reference remains weak.
#[derive(Clone)]
pub struct HistoryRegistration(Rc<RegisteredHistory>);

/// Cloneable allocation proof; cloned retained rows consume one slot together.
#[derive(Clone)]
pub struct HistoryPayloadPermit(Arc<PayloadAllocation>);
struct PayloadAllocation {
    order: u64,
}
impl HistoryPayloadPermit {
    pub fn same_allocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn order(&self) -> u64 {
        self.0.order
    }
}
/// An admitted pending buffer. Its immutable bytes stay charged until final drop.
#[derive(Clone)]
pub struct HistoryBytesPermit(Arc<usize>);
impl HistoryBytesPermit {
    pub fn bytes(&self) -> usize {
        *self.0
    }
}
#[derive(Default)]
struct AdmissionState {
    participants: Vec<Weak<RegisteredHistory>>,
    payloads: Vec<SyncWeak<PayloadAllocation>>,
    buffers: Vec<SyncWeak<usize>>,
    snapshots: Vec<local_allocations::TrackedSnapshot>,
    closed: bool,
    payload_order: u64,
}
struct InstallationHistoryAdmission {
    state: RefCell<AdmissionState>,
    admitting: Cell<bool>,
}
/// An explicit handle to the existing retained installation, shared by every partition.
#[derive(Clone)]
pub struct HistoryAdmissionHandle(Rc<InstallationHistoryAdmission>);

enum Requester {
    None,
    Entity(EntityId),
    Entities(Vec<EntityId>),
    Observed {
        registration: *const RegisteredHistory,
        entities: Vec<EntityId>,
    },
    Registration(*const RegisteredHistory),
}

struct AdmissionTurn<'a>(&'a Cell<bool>);
impl Drop for AdmissionTurn<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
impl HistoryAdmissionHandle {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new() -> Self {
        Self::with_initial_payload(false).0
    }
    /// Installation construction can move one already prepared body batch into
    /// its very first generation. No existing owner/quota can race this step.
    pub(crate) fn with_initial_payload(prepared: bool) -> (Self, Option<HistoryPayloadPermit>) {
        let allocation = prepared.then(|| Arc::new(PayloadAllocation { order: 1 }));
        let state = AdmissionState {
            payloads: allocation.iter().map(Arc::downgrade).collect(),
            payload_order: u64::from(prepared),
            ..AdmissionState::default()
        };
        let handle = Self(Rc::new(InstallationHistoryAdmission {
            state: RefCell::new(state),
            admitting: Cell::new(false),
        }));
        (handle, allocation.map(HistoryPayloadPermit))
    }
    pub fn register(
        &self,
        participant: Rc<dyn HistoryParticipant>,
    ) -> Result<HistoryRegistration, HistoryAdmissionError> {
        let mut state = self.0.state.borrow_mut();
        if state.closed {
            return Err(HistoryAdmissionError::Closed);
        }
        state.participants.retain(|entry| entry.strong_count() > 0);
        let registration = Rc::new(RegisteredHistory {
            owner: Rc::downgrade(&self.0),
            participant,
        });
        state.participants.push(Rc::downgrade(&registration));
        Ok(HistoryRegistration(registration))
    }
    pub fn reserve_payload(&self) -> Result<HistoryPayloadPermit, HistoryAdmissionError> {
        self.reserve_payload_to(MAX_NORMAL_HISTORY_PAYLOADS)
    }
    fn reserve_payload_to(
        &self,
        limit: usize,
    ) -> Result<HistoryPayloadPermit, HistoryAdmissionError> {
        let mut state = self.0.state.borrow_mut();
        if state.closed {
            return Err(HistoryAdmissionError::Closed);
        }
        state
            .snapshots
            .retain(local_allocations::TrackedSnapshot::alive);
        state.payloads.retain(|entry| entry.strong_count() > 0);
        if state.payloads.len() >= limit {
            return Err(HistoryAdmissionError::PayloadCapacity);
        }
        let order = state
            .payload_order
            .checked_add(1)
            .ok_or(HistoryAdmissionError::PayloadCapacity)?;
        state.payload_order = order;
        let allocation = Arc::new(PayloadAllocation { order });
        state.payloads.push(Arc::downgrade(&allocation));
        Ok(HistoryPayloadPermit(allocation))
    }
    /// Reclaim across Local and Cloud using one installation allocation order.
    /// Skip the entity currently borrowed by the caller. All callbacks run
    /// outside the budget's RefCell borrow, and nested decisions fail closed.
    pub fn reserve_payload_reclaiming(
        &self,
        requester: Option<EntityId>,
        cx: &mut App,
    ) -> Result<HistoryPayloadPermit, HistoryAdmissionError> {
        self.reserve_payload_excluding(&requester.into_iter().collect::<Vec<_>>(), cx)
    }
    fn reserve_payload_excluding(
        &self,
        excluded: &[EntityId],
        cx: &mut App,
    ) -> Result<HistoryPayloadPermit, HistoryAdmissionError> {
        if self.0.admitting.replace(true) {
            return Err(HistoryAdmissionError::Reentrant);
        }
        let _turn = AdmissionTurn(&self.0.admitting);
        let mut attempted = HashSet::new();
        let maximum_attempts = self
            .observations()?
            .0
            .len()
            .saturating_add(MAX_HISTORY_PAYLOADS);
        loop {
            match self.reserve_payload() {
                Ok(permit) => return Ok(permit),
                Err(HistoryAdmissionError::PayloadCapacity) => {}
                Err(error) => return Err(error),
            }
            let (participants, _) = self.observations()?;
            let victim = participants
                .into_iter()
                .filter(|entry| {
                    entry
                        .participant
                        .entity_id()
                        .is_none_or(|id| !excluded.contains(&id))
                })
                .filter_map(|entry| {
                    entry
                        .participant
                        .oldest_payload(cx)
                        .filter(|order| !attempted.contains(&(Rc::as_ptr(&entry), *order)))
                        .map(|order| (entry, order))
                })
                .min_by_key(|(_, order)| *order)
                .ok_or(HistoryAdmissionError::PayloadCapacity)?;
            if attempted.len() >= maximum_attempts {
                return Err(HistoryAdmissionError::PayloadCapacity);
            }
            attempted.insert((Rc::as_ptr(&victim.0), victim.1));
            victim.0.participant.evict_payload(cx);
            // Re-observe live allocations, including caller-retained rows. A
            // allocation ordinal is tried once, even if a callback reports success
            // without releasing it. At most the32live payloads can be reclaimed.
        }
    }
    pub fn reserve_bytes(
        &self,
        requester: &HistoryRegistration,
        bytes: usize,
        cx: &mut App,
    ) -> Result<HistoryBytesPermit, HistoryAdmissionError> {
        let owner = requester
            .0
            .owner
            .upgrade()
            .ok_or(HistoryAdmissionError::Closed)?;
        if !Rc::ptr_eq(&owner, &self.0) {
            return Err(HistoryAdmissionError::ForeignRegistration);
        }
        self.reserve(Requester::Registration(Rc::as_ptr(&requester.0)), bytes, cx)
    }
    /// Move a previously observed Local prepared body into a proof-backed
    /// buffer. The remaining source plus destination are admitted together.
    fn reserve_observed_buffer(
        &self,
        requester: &HistoryRegistration,
        entities: Vec<EntityId>,
        remaining: usize,
        bytes: usize,
        cx: &mut App,
    ) -> Result<HistoryBytesPermit, HistoryAdmissionError> {
        let owner = requester
            .0
            .owner
            .upgrade()
            .ok_or(HistoryAdmissionError::Closed)?;
        if !Rc::ptr_eq(&owner, &self.0) {
            return Err(HistoryAdmissionError::ForeignRegistration);
        }
        self.decide(
            Requester::Observed {
                registration: Rc::as_ptr(&requester.0),
                entities,
            },
            remaining.saturating_add(bytes),
            cx,
        )?;
        self.charge(bytes)
    }
    fn require_payload(&self, payload: &HistoryPayloadPermit) -> Result<(), HistoryAdmissionError> {
        let identity = Arc::downgrade(&payload.0);
        if self
            .0
            .state
            .borrow()
            .payloads
            .iter()
            .any(|entry| entry.ptr_eq(&identity))
        {
            Ok(())
        } else {
            Err(HistoryAdmissionError::ForeignAllocation)
        }
    }
    /// Additional temporary work while the requester is mutably borrowed. Its
    /// supplied existing bytes stay counted, but only the new buffer is charged
    /// by the returned token. No requester callback is invoked.
    pub fn reserve_buffer(
        &self,
        requester: &HistoryRegistration,
        existing: usize,
        bytes: usize,
        cx: &mut App,
    ) -> Result<HistoryBytesPermit, HistoryAdmissionError> {
        let owner = requester
            .0
            .owner
            .upgrade()
            .ok_or(HistoryAdmissionError::Closed)?;
        if !Rc::ptr_eq(&owner, &self.0) {
            return Err(HistoryAdmissionError::ForeignRegistration);
        }
        self.decide(
            Requester::Registration(Rc::as_ptr(&requester.0)),
            existing.saturating_add(bytes),
            cx,
        )?;
        self.charge(bytes)
    }
    /// Admission before a new entity or retained native result is materialized.
    pub fn reserve_unowned_bytes(
        &self,
        bytes: usize,
        cx: &mut App,
    ) -> Result<HistoryBytesPermit, HistoryAdmissionError> {
        self.reserve(Requester::None, bytes, cx)
    }
    /// Native metadata work is reserved while its owning Runtime is borrowed.
    /// Other installations/participants remain visible to this same decision.
    pub(crate) fn reserve_entity_bytes(
        &self,
        requester: EntityId,
        bytes: usize,
        cx: &mut App,
    ) -> Result<HistoryBytesPermit, HistoryAdmissionError> {
        self.reserve(Requester::Entity(requester), bytes, cx)
    }
    /// Transfer a unique reservation to a smaller immutable allocation. Shared
    /// reservations retain their conservative original charge until last drop.
    pub(crate) fn shrink_buffer(
        &self,
        allocation: HistoryBytesPermit,
        bytes: usize,
    ) -> Result<HistoryBytesPermit, HistoryAdmissionError> {
        if bytes > allocation.bytes() {
            return Err(HistoryAdmissionError::AllocationGrowth {
                reserved: allocation.bytes(),
                requested: bytes,
            });
        }
        let mut state = self.0.state.borrow_mut();
        if state.closed {
            return Err(HistoryAdmissionError::Closed);
        }
        let original = Arc::downgrade(&allocation.0);
        if !state.buffers.iter().any(|entry| entry.ptr_eq(&original)) {
            return Err(HistoryAdmissionError::ForeignAllocation);
        }
        if Arc::strong_count(&allocation.0) > 1 {
            return Ok(allocation);
        }
        let smaller = Arc::new(bytes);
        state.buffers.push(Arc::downgrade(&smaller));
        // No callback, await or externally visible decision occurs between the
        // replacement and releasing its old proof. Both byte counts are immutable.
        drop(state);
        drop(allocation);
        Ok(HistoryBytesPermit(smaller))
    }
    fn reserve(
        &self,
        requester: Requester,
        bytes: usize,
        cx: &mut App,
    ) -> Result<HistoryBytesPermit, HistoryAdmissionError> {
        self.decide(requester, bytes, cx)?;
        self.charge(bytes)
    }
    fn charge(&self, bytes: usize) -> Result<HistoryBytesPermit, HistoryAdmissionError> {
        let mut state = self.0.state.borrow_mut();
        if state.closed {
            return Err(HistoryAdmissionError::Closed);
        }
        let allocation = Arc::new(bytes);
        state.buffers.push(Arc::downgrade(&allocation));
        Ok(HistoryBytesPermit(allocation))
    }
    fn observations(&self) -> Result<(Vec<Rc<RegisteredHistory>>, usize), HistoryAdmissionError> {
        let mut state = self.0.state.borrow_mut();
        if state.closed {
            return Err(HistoryAdmissionError::Closed);
        }
        state.participants.retain(|entry| entry.strong_count() > 0);
        state
            .snapshots
            .retain(local_allocations::TrackedSnapshot::alive);
        state.buffers.retain(|entry| entry.strong_count() > 0);
        let participants = state
            .participants
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        let buffers = state
            .buffers
            .iter()
            .filter_map(SyncWeak::upgrade)
            .fold(0usize, |total, bytes| total.saturating_add(*bytes));
        Ok((participants, buffers))
    }
    /// Local admissions and Cloud reservations use exactly this same decision.
    pub(crate) fn admit(
        &self,
        requester: Option<EntityId>,
        incoming: usize,
        cx: &mut App,
    ) -> Result<Vec<ChatKey>, HistoryAdmissionError> {
        self.decide(
            requester.map_or(Requester::None, Requester::Entity),
            incoming,
            cx,
        )
    }
    fn decide(
        &self,
        requester: Requester,
        incoming: usize,
        cx: &mut App,
    ) -> Result<Vec<ChatKey>, HistoryAdmissionError> {
        self.decide_to(requester, incoming, MAX_NORMAL_HISTORY_BYTES, cx)
    }
    fn decide_to(
        &self,
        requester: Requester,
        incoming: usize,
        limit: usize,
        cx: &mut App,
    ) -> Result<Vec<ChatKey>, HistoryAdmissionError> {
        if self.0.admitting.replace(true) {
            return Err(HistoryAdmissionError::Reentrant);
        }
        let _turn = AdmissionTurn(&self.0.admitting);
        let mut attempted = HashSet::new();
        let mut evicted = Vec::new();
        let maximum_attempts = self
            .observations()?
            .0
            .len()
            .saturating_add(MAX_HISTORY_PAYLOADS);
        loop {
            let (participants, mut retained) = self.observations()?;
            let mut candidates = Vec::new();
            for participant in participants {
                let is_requester = match &requester {
                    Requester::None => false,
                    Requester::Entity(id) => participant.participant.entity_id() == Some(*id),
                    Requester::Entities(ids) => participant
                        .participant
                        .entity_id()
                        .is_some_and(|id| ids.contains(&id)),
                    Requester::Registration(ptr) => Rc::as_ptr(&participant) == *ptr,
                    Requester::Observed {
                        registration,
                        entities,
                    } => {
                        Rc::as_ptr(&participant) == *registration
                            || participant
                                .participant
                                .entity_id()
                                .is_some_and(|id| entities.contains(&id))
                    }
                };
                if is_requester {
                    continue;
                }
                let usage = participant.participant.usage(cx)?;
                if let Some(usage) = usage {
                    retained = retained.saturating_add(usage.bytes());
                }
                if let Some(candidate) = EvictionCandidate::new(participant, usage, cx)
                    && !attempted.contains(&candidate.identity())
                {
                    candidates.push(candidate);
                }
            }
            if self.0.state.borrow().closed {
                return Err(HistoryAdmissionError::Closed);
            }
            if retained.saturating_add(incoming) <= limit {
                return Ok(evicted);
            }
            candidates.sort_by_key(EvictionCandidate::priority);
            let Some(victim) = candidates
                .into_iter()
                .next()
                .filter(|_| attempted.len() < maximum_attempts)
            else {
                return Err(HistoryAdmissionError::ResidentCapacity {
                    retained,
                    incoming,
                    limit,
                });
            };
            attempted.insert(victim.identity());
            if let Some(key) = victim.evict(cx) {
                evicted.push(key);
            }
            // Re-observe actual data and retained buffers after the callback. A
            // callback may release registrations or immutable writer generations.
        }
    }
    /// A bounded allowance for an already captured urgent control. The service
    /// selects this only for original Stop, never normal prompt/data allocations.
    /// It consumes the same hard32slots/64MiB; it cannot extend either bound.
    pub fn reserve_urgent_control(
        &self,
        requester: &HistoryRegistration,
        bytes: usize,
        payloads: usize,
        cx: &mut App,
    ) -> Result<nocterm_core::allocation::AllocationLease, HistoryAdmissionError> {
        if payloads > 2 {
            return Err(HistoryAdmissionError::PayloadCapacity);
        }
        if bytes > URGENT_HISTORY_BYTES {
            return Err(HistoryAdmissionError::ResidentCapacity {
                retained: 0,
                incoming: bytes,
                limit: URGENT_HISTORY_BYTES,
            });
        }
        let owner = requester
            .0
            .owner
            .upgrade()
            .ok_or(HistoryAdmissionError::Closed)?;
        if !Rc::ptr_eq(&owner, &self.0) {
            return Err(HistoryAdmissionError::ForeignRegistration);
        }
        let mut slots = Vec::with_capacity(payloads);
        for _ in 0..payloads {
            slots.push(self.reserve_payload_to(MAX_HISTORY_PAYLOADS)?);
        }
        self.decide_to(
            Requester::Registration(Rc::as_ptr(&requester.0)),
            bytes,
            MAX_RESIDENT_BYTES,
            cx,
        )?;
        let proof = self.charge(bytes)?;
        Ok(nocterm_core::allocation::AllocationLease::new(
            bytes,
            payloads,
            (proof, slots),
        ))
    }
    pub(crate) fn close(&self) {
        self.0.state.borrow_mut().closed = true;
    }
}
