//! Receipt owner for actual metadata retained in StreamingDecoder::info.
use fdeflate::budget::{Allocation, AllocationBudget, AllocationRefused};

pub(super) struct MetadataBudget<B: AllocationBudget> {
    retained: Allocation<B>,
    pub text: Allocation<B>,
    pub ztxt: Allocation<B>,
    pub itxt: Allocation<B>,
}
impl<B: AllocationBudget> MetadataBudget<B> {
    pub fn new(budget: B) -> Result<Self, AllocationRefused> {
        Ok(Self {
            retained: Allocation::new(budget.clone(), 0)?,
            text: Allocation::new(budget.clone(), 0)?,
            ztxt: Allocation::new(budget.clone(), 0)?,
            itxt: Allocation::new(budget, 0)?,
        })
    }
    pub fn retain(&mut self, credit: Allocation<B>) {
        self.retained.absorb(credit);
    }
}
