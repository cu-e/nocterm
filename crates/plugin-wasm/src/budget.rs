//! How many guests may be alive at once.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use nocterm_plugin_abi::MAX_LIVE_INSTANCES;
use nocterm_plugin_api::RuntimeError;

/// The count of live instances, shared by the runtime and every slot it hands out.
#[derive(Clone, Default)]
pub(crate) struct Budget(Arc<AtomicUsize>);

/// One live instance. Dropping it frees the place.
pub(crate) struct LiveSlot(Arc<AtomicUsize>);

impl Budget {
    /// Takes a place, or `Capacity` when [`MAX_LIVE_INSTANCES`] are alive.
    pub(crate) fn claim(&self) -> Result<LiveSlot, RuntimeError> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
                (live < MAX_LIVE_INSTANCES).then_some(live + 1)
            })
            .map_err(|_| RuntimeError::Capacity)?;
        Ok(LiveSlot(Arc::clone(&self.0)))
    }
}

impl Drop for LiveSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
