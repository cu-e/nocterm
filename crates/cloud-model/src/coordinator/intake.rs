//! Real bounded reservations for native receive scratch, commands and ACK metadata.
//!
//! Reservation is independent of authentication and grants no command authority.
//! Native producers reserve known lengths before allocating their corresponding
//! plaintext/DTO generation; idle receive handles reserve no bytes.
use nocterm_core::allocation::AllocationLease;
use std::sync::Arc;
pub(crate) mod pool;
use pool::Pool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntakePoolLimits {
    bytes: usize,
    payloads: usize,
    allocations: usize,
}
impl IntakePoolLimits {
    pub fn new(bytes: usize, payloads: usize, allocations: usize) -> Result<Self, IntakeError> {
        if bytes == 0 || payloads == 0 || allocations == 0 || allocations > 4096 || payloads > 4096
        {
            return Err(IntakeError::InvalidLimits);
        }
        Ok(Self {
            bytes,
            payloads,
            allocations,
        })
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
    pub fn payloads(self) -> usize {
        self.payloads
    }
    pub fn allocations(self) -> usize {
        self.allocations
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntakeLane {
    Ingress,
    Ordinary,
    Urgent,
    Acknowledgement,
    UrgentAcknowledgement,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntakeLimits {
    ingress: IntakePoolLimits,
    ordinary: IntakePoolLimits,
    urgent: IntakePoolLimits,
    acknowledgement: IntakePoolLimits,
    urgent_acknowledgement: IntakePoolLimits,
}
impl IntakeLimits {
    pub fn new(
        ingress: IntakePoolLimits,
        ordinary: IntakePoolLimits,
        urgent: IntakePoolLimits,
        acknowledgement: IntakePoolLimits,
        urgent_acknowledgement: IntakePoolLimits,
    ) -> Result<Self, IntakeError> {
        ingress
            .bytes
            .checked_add(ordinary.bytes)
            .and_then(|total| total.checked_add(urgent.bytes))
            .and_then(|total| total.checked_add(acknowledgement.bytes))
            .and_then(|total| total.checked_add(urgent_acknowledgement.bytes))
            .ok_or(IntakeError::InvalidLimits)?;
        Ok(Self {
            ingress,
            ordinary,
            urgent,
            acknowledgement,
            urgent_acknowledgement,
        })
    }
    pub fn pool(self, lane: IntakeLane) -> IntakePoolLimits {
        match lane {
            IntakeLane::Ingress => self.ingress,
            IntakeLane::Ordinary => self.ordinary,
            IntakeLane::Urgent => self.urgent,
            IntakeLane::Acknowledgement => self.acknowledgement,
            IntakeLane::UrgentAcknowledgement => self.urgent_acknowledgement,
        }
    }
}
impl Default for IntakeLimits {
    fn default() -> Self {
        Self {
            ingress: IntakePoolLimits {
                bytes: 128 * 1024 * 1024,
                payloads: 4,
                allocations: 4,
            },
            ordinary: IntakePoolLimits {
                bytes: 128 * 1024 * 1024,
                payloads: 64,
                allocations: 64,
            },
            urgent: IntakePoolLimits {
                bytes: 2 * 1024 * 1024,
                payloads: 16,
                allocations: 16,
            },
            acknowledgement: IntakePoolLimits {
                bytes: 1024 * 1024,
                payloads: 64,
                allocations: 64,
            },
            urgent_acknowledgement: IntakePoolLimits {
                bytes: 256 * 1024,
                payloads: 16,
                allocations: 16,
            },
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntakeUsage {
    pub bytes: usize,
    pub payloads: usize,
    pub allocations: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IntakeError {
    #[error("the finite receive limits are invalid")]
    InvalidLimits,
    #[error("the bounded receive lane has no remaining capacity")]
    Capacity,
    #[error("the allocation generation counter is exhausted")]
    Exhausted,
    #[error("the receive allocation owner is unavailable")]
    Unavailable,
}
/// Lightweight shared handle. Cloning a handle does not reserve any capacity.
#[derive(Clone)]
pub struct CommandReceiveBudget {
    ingress: Arc<Pool>,
    ordinary: Arc<Pool>,
    urgent: Arc<Pool>,
    acknowledgement: Arc<Pool>,
    urgent_acknowledgement: Arc<Pool>,
}
impl CommandReceiveBudget {
    pub fn new(limits: IntakeLimits) -> Self {
        Self {
            ingress: Pool::new(limits.ingress),
            ordinary: Pool::new(limits.ordinary),
            urgent: Pool::new(limits.urgent),
            acknowledgement: Pool::new(limits.acknowledgement),
            urgent_acknowledgement: Pool::new(limits.urgent_acknowledgement),
        }
    }
    /// Allocation ownership only; neither equality nor this handle grants auth.
    pub fn same_owner(&self, other: &Self) -> bool {
        [
            IntakeLane::Ingress,
            IntakeLane::Ordinary,
            IntakeLane::Urgent,
            IntakeLane::Acknowledgement,
            IntakeLane::UrgentAcknowledgement,
        ]
        .into_iter()
        .all(|lane| Arc::ptr_eq(self.pool(lane), other.pool(lane)))
    }
    pub fn limits(&self, lane: IntakeLane) -> IntakePoolLimits {
        self.pool(lane).limits()
    }
    /// Reserve before allocating the actual generation. The lease must be held
    /// through all corresponding buffers/native tasks, including cancellation.
    pub fn try_reserve(
        &self,
        lane: IntakeLane,
        bytes: usize,
        payloads: usize,
    ) -> Result<AllocationLease, IntakeError> {
        self.pool(lane).reserve(bytes, payloads)
    }
    pub fn usage(&self, lane: IntakeLane) -> Result<IntakeUsage, IntakeError> {
        self.pool(lane).usage()
    }
    fn pool(&self, lane: IntakeLane) -> &Arc<Pool> {
        match lane {
            IntakeLane::Ingress => &self.ingress,
            IntakeLane::Ordinary => &self.ordinary,
            IntakeLane::Urgent => &self.urgent,
            IntakeLane::Acknowledgement => &self.acknowledgement,
            IntakeLane::UrgentAcknowledgement => &self.urgent_acknowledgement,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn budget() -> Result<CommandReceiveBudget, IntakeError> {
        let pool = IntakePoolLimits::new(16, 2, 2)?;
        Ok(CommandReceiveBudget::new(IntakeLimits::new(
            pool, pool, pool, pool, pool,
        )?))
    }
    #[test]
    fn ordinary_pressure_preserves_reserved_control_and_receive_scratch() -> Result<(), IntakeError>
    {
        let budget = budget()?;
        let normal = budget.try_reserve(IntakeLane::Ordinary, 16, 2)?;
        assert!(matches!(
            budget.try_reserve(IntakeLane::Ordinary, 1, 0),
            Err(IntakeError::Capacity)
        ));
        let _ingress = budget.try_reserve(IntakeLane::Ingress, 16, 1)?;
        let _urgent = budget.try_reserve(IntakeLane::Urgent, 16, 1)?;
        let _ack = budget.try_reserve(IntakeLane::Acknowledgement, 16, 0)?;
        drop(normal);
        assert_eq!(budget.usage(IntakeLane::Ordinary)?.allocations, 0);
        Ok(())
    }
    #[test]
    fn same_descriptive_counts_keep_distinct_generations_until_actual_last_drop()
    -> Result<(), IntakeError> {
        let budget = budget()?;
        let first = budget.try_reserve(IntakeLane::Ordinary, 8, 1)?;
        let old_native = first.clone();
        let second = budget.try_reserve(IntakeLane::Ordinary, 8, 1)?;
        drop(first);
        assert_eq!(budget.usage(IntakeLane::Ordinary)?.bytes, 16);
        drop(old_native);
        let third = budget.try_reserve(IntakeLane::Ordinary, 8, 1)?;
        drop(second);
        assert_eq!(budget.usage(IntakeLane::Ordinary)?.bytes, 8);
        drop(third);
        assert_eq!(budget.usage(IntakeLane::Ordinary)?.bytes, 0);
        Ok(())
    }
    #[test]
    fn zero_sized_metadata_still_consumes_a_bounded_generation_slot() -> Result<(), IntakeError> {
        let budget = budget()?;
        let _first = budget.try_reserve(IntakeLane::Acknowledgement, 0, 0)?;
        let _second = budget.try_reserve(IntakeLane::Acknowledgement, 0, 0)?;
        assert!(matches!(
            budget.try_reserve(IntakeLane::Acknowledgement, 0, 0),
            Err(IntakeError::Capacity)
        ));
        assert!(matches!(
            budget.try_reserve(IntakeLane::Ingress, usize::MAX, 1),
            Err(IntakeError::Capacity)
        ));
        Ok(())
    }
}
