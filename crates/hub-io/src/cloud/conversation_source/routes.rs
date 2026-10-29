//! Bounded exact logical-cursor routing across canceled observer deliveries.
//! A physically persisted/offered page need not have reached a view. Origin is
//! always routable; recent exact cursor hashes map to actual relay positions.
use super::{MAX_WIRE_BYTES, heads, requirements};
use nocterm_cloud_model::projection::{
    ProjectionCursor, ProjectionError, ports::AllocationRequirements,
};
use std::collections::VecDeque;
const RECENT: usize = 63;
#[derive(Clone, Copy)]
struct Offered {
    cursor: ProjectionCursor,
    delivery: u64,
}
pub(super) struct ReplayRoutes {
    high: Offered,
    recent: VecDeque<Offered>,
}
impl ReplayRoutes {
    pub(super) fn new() -> Self {
        Self {
            high: Offered {
                cursor: ProjectionCursor::origin(),
                delivery: 0,
            },
            recent: VecDeque::with_capacity(RECENT),
        }
    }
    pub(super) fn high(&self) -> ProjectionCursor {
        self.high.cursor
    }
    pub(super) fn delivered(&self) -> u64 {
        self.high.delivery
    }
    pub(super) fn delivery(&self, after: ProjectionCursor) -> Result<u64, ProjectionError> {
        if after == ProjectionCursor::origin() {
            return Ok(0);
        }
        if after == self.high.cursor {
            return Ok(self.high.delivery);
        }
        if let Some(offered) = self.recent.iter().find(|old| old.cursor == after) {
            return Ok(offered.delivery);
        }
        if after.sequence() == self.high.cursor.sequence()
            || self
                .recent
                .iter()
                .any(|old| old.cursor.sequence() == after.sequence())
        {
            return Err(ProjectionError::Rollback);
        }
        // No reconstruction from an unrecognized sparse position. Rebind/restore
        // a real snapshot when that separate transition becomes supported.
        Err(ProjectionError::Unsupported)
    }
    pub(super) fn requirements(
        &self,
        after: ProjectionCursor,
        known: ProjectionCursor,
    ) -> Result<AllocationRequirements, ProjectionError> {
        self.delivery(after)?;
        if known.sequence() < after.sequence()
            || (known.sequence() == after.sequence() && known != after)
        {
            return Err(ProjectionError::Rollback);
        }
        let bytes = if known == after {
            heads::work_bytes()
        } else {
            requirements::replay(MAX_WIRE_BYTES)?
        };
        Ok(AllocationRequirements::new(bytes, 1))
    }
    /// Called only after authenticated page persistence. Re-offering the exact
    /// same canceled page is idempotent and never reduces verified highwater.
    pub(super) fn offer(
        &mut self,
        after: ProjectionCursor,
        next: ProjectionCursor,
        delivery: u64,
    ) -> Result<(), ProjectionError> {
        let previous = self.delivery(after)?;
        if next.sequence() <= after.sequence() || delivery <= previous {
            return Err(ProjectionError::Discontinuous);
        }
        if let Some(old) = self
            .recent
            .iter()
            .find(|old| old.cursor.sequence() == next.sequence())
        {
            if old.cursor != next || old.delivery != delivery {
                return Err(ProjectionError::Rollback);
            }
            return Ok(());
        }
        if next.sequence() == self.high.cursor.sequence() {
            if next != self.high.cursor || delivery != self.high.delivery {
                return Err(ProjectionError::Rollback);
            }
        } else if next.sequence() > self.high.cursor.sequence() {
            if delivery <= self.high.delivery {
                return Err(ProjectionError::Rollback);
            }
            self.high = Offered {
                cursor: next,
                delivery,
            };
        } else if delivery >= self.high.delivery {
            return Err(ProjectionError::Rollback);
        }
        self.recent.push_back(Offered {
            cursor: next,
            delivery,
        });
        if self.recent.len() > RECENT {
            self.recent.pop_front();
        }
        Ok(())
    }
}
