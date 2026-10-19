use crate::{
    model::{
        notice::UsageNotice,
        usage::{ContextUsage, TokenTotals},
    },
    usage::{Limits, RateLimit},
};
#[derive(Clone, Debug, Default)]
pub(super) struct ThreadUsage {
    context: Option<ContextUsage>,
    tokens: Option<TokenTotals>,
    limits: Limits,
}
impl ThreadUsage {
    pub(super) fn context(&self) -> Option<&ContextUsage> {
        self.context.as_ref()
    }
    pub(super) fn tokens(&self) -> Option<&TokenTotals> {
        self.tokens.as_ref()
    }
    pub(super) fn limits(&self) -> impl Iterator<Item = &RateLimit> {
        self.limits.iter()
    }
    pub(super) fn finish(&mut self, tokens: Option<TokenTotals>) {
        self.tokens = tokens;
    }
    pub(super) fn observe(&mut self, notice: UsageNotice) {
        self.context = Some(notice.context);
        for limit in notice.rate_limits {
            self.limits.merge(limit);
        }
    }
}
