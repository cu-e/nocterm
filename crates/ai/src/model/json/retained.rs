//! Authentic borrowed allocation components; no reservation or caller cost is minted.
use super::{JsonReceipt, deferred::DecodeLedger};

#[derive(Clone, Copy)]
enum Allocation<'a> {
    Root(&'a JsonReceipt),
    Ledger(&'a DecodeLedger),
}
/// Immutable identity and requested bytes of one actual retained allocation bundle.
/// Constructors and owner kinds stay private. Comparing components neither clones
/// a payload nor authorizes a caller-supplied identity, address or byte count.
#[derive(Clone, Copy)]
pub struct RetainedAllocation<'a> {
    allocation: Allocation<'a>,
}
impl<'a> RetainedAllocation<'a> {
    pub(super) fn root(receipt: &'a JsonReceipt) -> Self {
        Self {
            allocation: Allocation::Root(receipt),
        }
    }
    pub(super) fn ledger(ledger: &'a DecodeLedger) -> Self {
        Self {
            allocation: Allocation::Ledger(ledger),
        }
    }
    /// Root bytes preserve the conservative in-progress Drop query. Ledger bytes
    /// describe only its fixed allocation, never unrelated siblings' live usage.
    pub fn bytes(&self) -> Option<usize> {
        match self.allocation {
            Allocation::Root(receipt) => receipt.retained_identity_bytes(),
            Allocation::Ledger(ledger) => Some(ledger.allocation_bytes()),
        }
    }
    pub fn same_allocation(&self, other: &Self) -> bool {
        match (self.allocation, other.allocation) {
            (Allocation::Root(left), Allocation::Root(right)) => {
                left.identity.ptr_eq(&right.identity)
            }
            (Allocation::Ledger(left), Allocation::Ledger(right)) => left.same_allocation(right),
            _ => false,
        }
    }
}
#[cfg(test)]
mod tests;
