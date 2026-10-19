//! Explicit native search scratch with real allocation-before-construction credit.
use super::{ActiveStates, Cache, FollowEpsilon, PikeVM, SlotTable};
use crate::{
    util::{
        primitives::{NonMaxUsize, StateID},
        sparse_set::SparseSet,
    },
    Input, PatternID,
};
use alloc::{collections::TryReserveError, vec::Vec};
use core::{alloc::Layout, fmt};

/// Authority supplied by the caller of a managed search. Credits must keep their
/// allocation liability alive until their physical backing is freed.
pub trait Budget {
    /// Opaque original allocation receipt.
    type Credit;
    /// Original typed reservation refusal.
    type Error;
    /// Reserve the actual requested backing layout before allocation.
    fn reserve(&self, layout: Layout) -> Result<Self::Credit, Self::Error>;
}
/// A managed search preserves both native refusals and actual allocator causes.
#[derive(Debug)]
pub enum Error<E> {
    /// Original native authority refused a backing layout.
    Refused(E),
    /// Original allocator refused storage after native admission.
    Allocate(TryReserveError),
    /// Requested capacity cannot be represented by a native allocation layout.
    Capacity,
    /// The allocator supplied a capacity different from the exact paid request.
    CapacityProfile,
}
impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(error) => write!(f, "managed regex reservation refused: {error}"),
            Self::Allocate(error) => write!(f, "managed regex allocation failed: {error}"),
            Self::Capacity => f.write_str("managed regex capacity overflow"),
            Self::CapacityProfile => {
                f.write_str("managed regex allocation capacity differs from its reservation")
            }
        }
    }
}
#[cfg(feature = "std")]
impl<E: std::error::Error + 'static> std::error::Error for Error<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(error) => Some(error),
            Self::Allocate(error) => Some(error),
            _ => None,
        }
    }
}
struct Paid<T, C> {
    values: Vec<T>,
    credit: Option<C>,
}
fn fixed<T: Clone, B: Budget>(
    budget: &B,
    len: usize,
    value: T,
) -> Result<Paid<T, B::Credit>, Error<B::Error>> {
    let layout = Layout::array::<T>(len).map_err(|_| Error::Capacity)?;
    let credit = budget.reserve(layout).map_err(Error::Refused)?;
    let mut values = Vec::new();
    values.try_reserve_exact(len).map_err(Error::Allocate)?;
    if core::mem::size_of::<T>() != 0 && values.capacity() != len {
        return Err(Error::CapacityProfile);
    }
    values.resize(len, value);
    Ok(Paid {
        values,
        credit: Some(credit),
    })
}
/// One non-cloneable scratch owner tied to the exact borrowed compiled engine.
/// All seven engine vectors and capture offsets are prepaid; actual cache and
/// capture storage are freed before any associated credit is dropped.
pub struct ManagedCache<'r, B: Budget> {
    cache: Cache,
    slots: Vec<Option<NonMaxUsize>>,
    _fixed_credits: [Option<B::Credit>; 7],
    stack_credit: Option<B::Credit>,
    re: &'r PikeVM,
    budget: B,
}
impl<B: Budget> fmt::Debug for ManagedCache<'_, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedCache")
            .field("memory_usage", &self.memory_usage())
            .finish_non_exhaustive()
    }
}
impl<'r, B: Budget> ManagedCache<'r, B> {
    /// Admit actual fixed state tables before constructing them. The dynamic
    /// epsilon stack starts empty and grows only for the actual search.
    pub fn new(re: &'r PikeVM, budget: B) -> Result<Self, Error<B::Error>> {
        let nfa = re.get_nfa();
        let states = nfa.states().len();
        let stride = nfa.group_info().slot_len();
        let captures = stride.max(nfa.pattern_len().checked_mul(2).ok_or(Error::Capacity)?);
        let table = states
            .checked_mul(stride)
            .and_then(|n| n.checked_add(captures))
            .ok_or(Error::Capacity)?;
        let mut a = fixed(&budget, states, StateID::ZERO)?;
        let mut b = fixed(&budget, states, StateID::ZERO)?;
        let mut c = fixed(&budget, table, None)?;
        let mut d = fixed(&budget, states, StateID::ZERO)?;
        let mut e = fixed(&budget, states, StateID::ZERO)?;
        let mut f = fixed(&budget, table, None)?;
        let mut slots = fixed(&budget, captures, None)?;
        let cache = Cache {
            stack: Vec::new(),
            curr: ActiveStates {
                set: SparseSet::prepared(
                    core::mem::take(&mut a.values),
                    core::mem::take(&mut b.values),
                ),
                slot_table: SlotTable {
                    table: core::mem::take(&mut c.values),
                    slots_per_state: stride,
                    slots_for_captures: captures,
                },
            },
            next: ActiveStates {
                set: SparseSet::prepared(
                    core::mem::take(&mut d.values),
                    core::mem::take(&mut e.values),
                ),
                slot_table: SlotTable {
                    table: core::mem::take(&mut f.values),
                    slots_per_state: stride,
                    slots_for_captures: captures,
                },
            },
        };
        Ok(Self {
            cache,
            slots: core::mem::take(&mut slots.values),
            _fixed_credits: [
                a.credit.take(),
                b.credit.take(),
                c.credit.take(),
                d.credit.take(),
                e.credit.take(),
                f.credit.take(),
                slots.credit.take(),
            ],
            stack_credit: None,
            re,
            budget,
        })
    }
    /// Search using the original PikeVM transitions and Unicode/leftmost rules.
    /// Capture offsets are borrowed with [`Self::slots`] after a successful match.
    pub fn search(&mut self, input: &Input<'_>) -> Result<Option<PatternID>, Error<B::Error>> {
        self.slots.fill(None);
        let mut growth = Controlled {
            budget: &self.budget,
            credit: &mut self.stack_credit,
            failure: None,
        };
        let found = self.re.search_slots_imp_controlled(
            &mut self.cache,
            input,
            &mut self.slots,
            &mut growth,
        );
        if let Some(error) = growth.failure {
            self.slots.fill(None);
            return Err(error);
        }
        Ok(found.map(|matched| matched.pattern()))
    }
    /// Borrow original capture offsets without allocating a capture object.
    pub fn slots(&self) -> &[Option<NonMaxUsize>] {
        &self.slots
    }
    /// Real requested backing bytes retained by this scratch owner.
    pub fn memory_usage(&self) -> usize {
        self.cache.stack.capacity() * core::mem::size_of::<FollowEpsilon>()
            + self.cache.curr.memory_usage()
            + self.cache.next.memory_usage()
            + self.slots.capacity() * core::mem::size_of::<Option<NonMaxUsize>>()
    }
}
pub(super) trait Growth {
    fn reserve(&mut self, stack: &mut Vec<FollowEpsilon>, additional: usize) -> bool;
    fn failed(&self) -> bool;
}
pub(super) struct Unmanaged;
impl Growth for Unmanaged {
    fn reserve(&mut self, _: &mut Vec<FollowEpsilon>, _: usize) -> bool {
        true
    }
    fn failed(&self) -> bool {
        false
    }
}
struct Controlled<'a, B: Budget> {
    budget: &'a B,
    credit: &'a mut Option<B::Credit>,
    failure: Option<Error<B::Error>>,
}
impl<B: Budget> Controlled<'_, B> {
    fn grow(
        &mut self,
        stack: &mut Vec<FollowEpsilon>,
        needed: usize,
    ) -> Result<(), Error<B::Error>> {
        let desired = stack
            .capacity()
            .checked_mul(2)
            .filter(|&n| Layout::array::<FollowEpsilon>(n).is_ok())
            .unwrap_or(needed)
            .max(needed);
        let (credit, capacity) = match self
            .budget
            .reserve(Layout::array::<FollowEpsilon>(desired).map_err(|_| Error::Capacity)?)
        {
            Ok(credit) => (credit, desired),
            Err(_) if desired != needed => (
                self.budget
                    .reserve(Layout::array::<FollowEpsilon>(needed).map_err(|_| Error::Capacity)?)
                    .map_err(Error::Refused)?,
                needed,
            ),
            Err(error) => return Err(Error::Refused(error)),
        };
        let mut new = Vec::new();
        new.try_reserve_exact(capacity).map_err(Error::Allocate)?;
        if new.capacity() != capacity {
            return Err(Error::CapacityProfile);
        }
        new.extend(stack.drain(..));
        let old = core::mem::replace(stack, new);
        drop(old);
        let old_credit = self.credit.replace(credit);
        drop(old_credit);
        Ok(())
    }
}
impl<B: Budget> Growth for Controlled<'_, B> {
    fn reserve(&mut self, stack: &mut Vec<FollowEpsilon>, additional: usize) -> bool {
        if self.failure.is_some() {
            return false;
        }
        let Some(needed) = stack.len().checked_add(additional) else {
            self.failure = Some(Error::Capacity);
            return false;
        };
        if needed <= stack.capacity() {
            return true;
        }
        match self.grow(stack, needed) {
            Ok(()) => true,
            Err(error) => {
                self.failure = Some(error);
                false
            }
        }
    }
    fn failed(&self) -> bool {
        self.failure.is_some()
    }
}

#[cfg(all(test, feature = "std"))]
mod tests;
