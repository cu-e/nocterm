//! Cached row budgets remain separate from paint invalidation and semantic activity.
use nocterm_ai::{
    history::{HistoryBudget, HistoryCodecError, PreparedBudget, SavedChat},
    thread::Entry,
};
use std::collections::HashSet;
#[derive(Default)]
pub(super) struct BudgetState {
    budget: HistoryBudget,
    dirty: HashSet<usize>,
}
impl BudgetState {
    pub(super) fn restored(prepared: Option<&PreparedBudget>) -> Self {
        Self {
            budget: HistoryBudget::restored(prepared),
            dirty: HashSet::new(),
        }
    }
    pub(super) fn mark(&mut self, row: usize) {
        self.dirty.insert(row);
    }
    pub(super) fn refresh(&mut self, entries: &[Entry]) -> Result<(), HistoryCodecError> {
        let dirty = self.dirty.iter().copied().collect::<Vec<_>>();
        self.budget.refresh(entries, &dirty)?;
        self.dirty.clear();
        Ok(())
    }
    pub(super) fn resident_bytes(&self) -> usize {
        self.budget.resident_bytes()
    }
    pub(super) fn validate(
        &self,
        metadata: &SavedChat,
        encoded: impl Iterator<Item = usize>,
        memory: impl Iterator<Item = usize>,
    ) -> Result<(), HistoryCodecError> {
        self.budget.validate_memory(metadata, memory)?;
        self.budget.validate(metadata, encoded)
    }
}
