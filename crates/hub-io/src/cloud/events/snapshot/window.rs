//! Bounded control/causal closure plus recent optional complete display facts.
mod facts;
use facts::CausalFacts;
use nocterm_cloud_model::journal::{JournalError, PublicationAllocation, PublicationWorkBudget};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_model::cloud::events::{CloudSnapshotPayload, MAX_SNAPSHOT_EVENTS};
use std::mem::{size_of, swap};

pub(super) struct WindowSelection {
    facts: CausalFacts,
    selected: Selection,
    trial: Selection,
    _allocation: AllocationLease,
}
impl WindowSelection {
    pub(super) fn retain(
        payload: &mut CloudSnapshotPayload<'_>,
        records: usize,
        bytes: usize,
        work: &dyn PublicationWorkBudget,
    ) -> Result<(), JournalError> {
        let count = payload.events.len();
        if count > 2 * MAX_SNAPSHOT_EVENTS {
            return Err(JournalError::Capacity);
        }
        let required = CausalFacts::storage_bytes(count)
            .and_then(|n| {
                Selection::storage_bytes(count)?
                    .checked_mul(2)?
                    .checked_add(n)
            })
            .ok_or(JournalError::Capacity)?;
        let allocation = work.try_reserve(PublicationAllocation::new(required.max(1), 1)?)?;
        let empty = CloudSnapshotPayload {
            control_revision: payload.control_revision,
            timestamp_ms: payload.timestamp_ms,
            omitted_earlier: payload.omitted_earlier,
            events: Vec::new(),
        };
        let overhead = empty
            .canonical_len()
            .map_err(super::super::encoding_error)?;
        if overhead > bytes {
            return Err(JournalError::Capacity);
        }
        let mut owner = Self {
            facts: CausalFacts::empty(count),
            selected: Selection::empty(count, overhead),
            trial: Selection::empty(count, overhead),
            _allocation: allocation,
        };
        owner.facts.index(&payload.events)?;
        owner.select(records.min(MAX_SNAPSHOT_EVENTS), bytes)?;
        let omitted = owner.selected.count != count;
        let mut index = 0;
        payload.events.retain(|_| {
            let keep = owner.selected.flags.get(index).copied().unwrap_or(false);
            index += 1;
            keep
        });
        payload.omitted_earlier |= omitted;
        if payload
            .canonical_len()
            .map_err(super::super::encoding_error)?
            != owner.selected.bytes
        {
            return Err(JournalError::Corrupt);
        }
        Ok(())
    }
    fn select(&mut self, records: usize, bytes: usize) -> Result<(), JournalError> {
        for index in 0..self.facts.count() {
            if self.facts.required(index)? {
                close(&self.facts, &mut self.selected, index, records, bytes)?;
            }
        }
        for index in (0..self.facts.count()).rev() {
            if self.selected.flags.get(index).copied().unwrap_or(false) {
                continue;
            }
            self.trial.reset(&self.selected);
            match close(&self.facts, &mut self.trial, index, records, bytes) {
                Ok(()) => swap(&mut self.selected, &mut self.trial),
                Err(JournalError::Capacity) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}
struct Selection {
    flags: Vec<bool>,
    pending: Vec<usize>,
    bytes: usize,
    count: usize,
}
impl Selection {
    fn storage_bytes(count: usize) -> Option<usize> {
        count.checked_mul(size_of::<bool>() + size_of::<usize>())
    }
    fn empty(count: usize, bytes: usize) -> Self {
        Self {
            flags: vec![false; count],
            pending: Vec::with_capacity(count),
            bytes,
            count: 0,
        }
    }
    fn reset(&mut self, previous: &Self) {
        self.flags.copy_from_slice(&previous.flags);
        self.pending.clear();
        self.bytes = previous.bytes;
        self.count = previous.count;
    }
    fn include(
        &mut self,
        index: usize,
        facts: &CausalFacts,
        records: usize,
        bytes: usize,
    ) -> Result<(), JournalError> {
        let flag = self.flags.get_mut(index).ok_or(JournalError::Corrupt)?;
        if *flag {
            return Ok(());
        }
        let next = self
            .bytes
            .checked_add(facts.bytes(index)?)
            .ok_or(JournalError::Capacity)?;
        if self.count >= records || next > bytes {
            return Err(JournalError::Capacity);
        }
        *flag = true;
        self.count += 1;
        self.bytes = next;
        self.pending.push(index);
        Ok(())
    }
}
fn close(
    facts: &CausalFacts,
    selected: &mut Selection,
    index: usize,
    records: usize,
    bytes: usize,
) -> Result<(), JournalError> {
    selected.include(index, facts, records, bytes)?;
    while let Some(index) = selected.pending.pop() {
        let (run, tool) = facts.links(index)?;
        if let Some(run) = run {
            for root in facts.run_roots(run)?.into_iter().flatten() {
                selected.include(root, facts, records, bytes)?;
            }
        }
        if let Some(tool) = tool {
            for root in facts.tool_roots(tool)?.into_iter().flatten() {
                selected.include(root, facts, records, bytes)?;
            }
        }
    }
    Ok(())
}
