//! Exact replacement storage keeps both old and new backing charged during growth.
use super::*;
use std::task::Waker;
struct Waiter {
    registered: bool,
    waker: Option<Waker>,
}
pub(super) struct Waiters {
    entries: Vec<Waiter>,
    allocation: Option<NativeAllocation>,
}
impl Waiters {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
            allocation: None,
        }
    }
    pub(super) fn register(
        &mut self,
        context: &SourceContext,
    ) -> Result<usize, NativeAllocationError> {
        if let Some((index, value)) = self
            .entries
            .iter_mut()
            .enumerate()
            .find(|(_, value)| !value.registered)
        {
            value.registered = true;
            return Ok(index);
        }
        let index = self.entries.len();
        let length = index
            .checked_add(1)
            .ok_or(NativeAllocationError::Arithmetic)?;
        let allocation = context.reserve_requested(
            Layout::array::<Waiter>(length).map_err(|_| NativeAllocationError::Arithmetic)?,
        )?;
        let mut entries = Vec::with_capacity(length);
        if entries.capacity() != length {
            return Err(NativeAllocationError::CapacityProfile);
        }
        // Moves do not clone executor wakers or their captured scheduling state.
        entries.append(&mut self.entries);
        entries.push(Waiter {
            registered: true,
            waker: None,
        });
        let old_entries = std::mem::replace(&mut self.entries, entries);
        drop(old_entries);
        drop(self.allocation.replace(allocation));
        Ok(index)
    }
    pub(super) fn update(&mut self, index: usize, waker: &Waker) {
        if let Some(value) = self.entries.get_mut(index)
            && value.registered
            && !value.waker.as_ref().is_some_and(|old| old.will_wake(waker))
        {
            value.waker = Some(waker.clone());
        }
    }
    pub(super) fn remove(&mut self, index: usize) {
        if let Some(value) = self.entries.get_mut(index) {
            value.registered = false;
            drop(value.waker.take());
        }
    }
    pub(super) fn wake(mut self) {
        for value in &mut self.entries {
            if let Some(waker) = value.waker.take() {
                waker.wake();
            }
        }
    }
}
