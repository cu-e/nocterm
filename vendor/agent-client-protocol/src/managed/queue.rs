//! Exact-layout FIFO storage with paid old/new backing overlap.
use super::{Budget, Buffer, ManagedError};
pub(super) struct Queue<T, B: Budget> {
    slots: Buffer<Option<T>, B>,
    head: usize,
    len: usize,
    budget: B,
}
impl<T, B: Budget> Queue<T, B> {
    pub(super) fn new(budget: B) -> Self {
        Self {
            slots: Buffer::new(budget.clone()),
            head: 0,
            len: 0,
            budget,
        }
    }
    pub(super) fn reserve(&mut self) -> Result<(), ManagedError<B::Error>> {
        if self.len < self.slots.len() {
            return Ok(());
        }
        let capacity = self
            .slots
            .len()
            .max(1)
            .checked_mul(2)
            .ok_or(ManagedError::CapacityOverflow)?;
        let slots_result = self.allocate_slots(capacity);
        let mut slots = match slots_result {
            Err(ManagedError::Refused(_)) if capacity > self.len + 1 => {
                self.allocate_slots(self.len + 1)?
            }
            result => result?,
        };
        for (index, target) in slots.as_mut_slice().iter_mut().take(self.len).enumerate() {
            let source = (self.head + index) % self.slots.len();
            *target = self.slots.as_mut_slice()[source].take();
        }
        self.slots = slots;
        self.head = 0;
        Ok(())
    }
    fn allocate_slots(
        &self,
        capacity: usize,
    ) -> Result<Buffer<Option<T>, B>, ManagedError<B::Error>> {
        let mut slots = Buffer::new(self.budget.clone());
        slots.reserve(capacity)?;
        for _ in 0..capacity {
            slots.push(None).map_err(|(error, _)| error)?;
        }
        Ok(slots)
    }
    pub(super) fn push_reserved(&mut self, value: T) {
        let position = (self.head + self.len) % self.slots.len();
        self.slots.as_mut_slice()[position] = Some(value);
        self.len += 1;
    }
    pub(super) fn front(&self) -> Option<&T> {
        if self.len == 0 { None } else { self.slots.as_slice()[self.head].as_ref() }
    }
    pub(super) fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = self.slots.as_mut_slice()[self.head].take();
        self.head = (self.head + 1) % self.slots.len();
        self.len -= 1;
        value
    }
}
