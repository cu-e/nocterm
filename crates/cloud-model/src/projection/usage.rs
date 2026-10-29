//! Each usage event replaces the complete optional cumulative run snapshot.
use crate::engine::UsageObservation;
use nocterm_ai::model::usage::TokenTotals;

#[derive(Clone, Default)]
pub(super) struct ProjectionUsage(UsageObservation);
impl ProjectionUsage {
    pub(super) fn observation(&self) -> UsageObservation {
        self.0
    }
    pub(super) fn observe(&mut self, next: UsageObservation) -> Option<TokenTotals> {
        self.0 = next;
        let mut totals = TokenTotals::new(
            self.0.total_tokens?,
            self.0.input_tokens?,
            self.0.output_tokens?,
        );
        totals.thought = self.0.thought_tokens;
        totals.cache_read = self.0.cached_input_tokens;
        Some(totals)
    }
}
