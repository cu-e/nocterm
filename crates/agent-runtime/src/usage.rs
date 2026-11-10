//! Plan limits retained from accepted ACP usage events.
use nocterm_ai::usage::{Limits, RateLimit};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct UsageLimits {
    limits: HashMap<String, Limits>,
}

impl UsageLimits {
    pub(super) fn get(&self, id: &str) -> Option<&Limits> {
        self.limits.get(id).filter(|limits| !limits.is_empty())
    }
    pub(super) fn merge(&mut self, id: String, limit: RateLimit) {
        self.limits.entry(id).or_default().merge(limit)
    }
}
