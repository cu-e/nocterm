//! Shared finite reservation generations used by intake and publication work.
use super::{IntakeError, IntakePoolLimits};
use nocterm_core::allocation::AllocationLease;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
pub(crate) struct Pool {
    limits: IntakePoolLimits,
    state: Mutex<Reservations>,
}
#[derive(Default)]
struct Reservations {
    next: u64,
    bytes: usize,
    payloads: usize,
    allocations: BTreeMap<u64, (usize, usize)>,
    healthy: bool,
}
impl Pool {
    pub(crate) fn new(limits: IntakePoolLimits) -> Arc<Self> {
        Arc::new(Self {
            limits,
            state: Mutex::new(Reservations {
                healthy: true,
                ..Reservations::default()
            }),
        })
    }
    pub(super) fn limits(&self) -> IntakePoolLimits {
        self.limits
    }
    pub(super) fn usage(&self) -> Result<super::IntakeUsage, IntakeError> {
        let state = self.state.lock().map_err(|_| IntakeError::Unavailable)?;
        if !state.healthy {
            return Err(IntakeError::Unavailable);
        }
        Ok(super::IntakeUsage {
            bytes: state.bytes,
            payloads: state.payloads,
            allocations: state.allocations.len(),
        })
    }
    pub(crate) fn reserve(
        self: &Arc<Self>,
        bytes: usize,
        payloads: usize,
    ) -> Result<AllocationLease, IntakeError> {
        let mut state = self.state.lock().map_err(|_| IntakeError::Unavailable)?;
        if !state.healthy {
            return Err(IntakeError::Unavailable);
        }
        let total_bytes = state
            .bytes
            .checked_add(bytes)
            .ok_or(IntakeError::Capacity)?;
        let total_payloads = state
            .payloads
            .checked_add(payloads)
            .ok_or(IntakeError::Capacity)?;
        if total_bytes > self.limits.bytes
            || total_payloads > self.limits.payloads
            || state.allocations.len() >= self.limits.allocations
        {
            return Err(IntakeError::Capacity);
        }
        let id = state.next.checked_add(1).ok_or(IntakeError::Exhausted)?;
        state.next = id;
        state.bytes = total_bytes;
        state.payloads = total_payloads;
        state.allocations.insert(id, (bytes, payloads));
        drop(state);
        Ok(AllocationLease::new(
            bytes,
            payloads,
            Reservation {
                owner: self.clone(),
                id,
            },
        ))
    }
}
struct Reservation {
    owner: Arc<Pool>,
    id: u64,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        // Poison or an impossible missing generation fails closed: no subsequent
        // allocation may reuse capacity whose real ownership is uncertain.
        let Ok(mut state) = self.owner.state.lock() else {
            return;
        };
        let Some((bytes, payloads)) = state.allocations.remove(&self.id) else {
            state.healthy = false;
            return;
        };
        match (
            state.bytes.checked_sub(bytes),
            state.payloads.checked_sub(payloads),
        ) {
            (Some(bytes), Some(payloads)) => {
                state.bytes = bytes;
                state.payloads = payloads;
            }
            _ => state.healthy = false,
        }
    }
}
