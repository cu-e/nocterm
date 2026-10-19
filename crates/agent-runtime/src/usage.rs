//! Retained usage projections and cancellable refresh ownership.
use super::Runtime;
use gpui::{Context, Task};
use nocterm_ai::usage::{AccountUsageSource, Limits, RateLimit, UsageAgent, UsageReadError};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
pub(super) struct UsageLimits {
    limits: HashMap<String, Limits>,
    reading: HashSet<String>,
    tasks: HashMap<String, Task<()>>,
    source: Arc<dyn AccountUsageSource>,
}
impl UsageLimits {
    pub(super) fn new(source: Arc<dyn AccountUsageSource>) -> Self {
        Self {
            source,
            limits: HashMap::new(),
            reading: HashSet::new(),
            tasks: HashMap::new(),
        }
    }
    pub(super) fn get(&self, id: &str) -> Option<&Limits> {
        self.limits.get(id).filter(|limits| !limits.is_empty())
    }
    pub(super) fn merge(&mut self, id: String, limit: RateLimit) {
        self.limits.entry(id).or_default().merge(limit)
    }
    fn complete(&mut self, id: String, limits: Result<Option<Vec<RateLimit>>, UsageReadError>) {
        self.reading.remove(&id);
        self.tasks.remove(&id);
        if let Ok(Some(limits)) = limits {
            self.limits.entry(id).or_default().replace(limits)
        }
    }
    pub(super) fn refresh(&mut self, agent: UsageAgent, cx: &mut Context<Runtime>) {
        let id = agent.id.clone();
        if !self.reading.insert(id.clone()) {
            return;
        }
        let source = self.source.clone();
        let reading = cx
            .background_executor()
            .spawn(async move { source.read(agent).await });
        let key = id.clone();
        let task = cx.spawn(async move |runtime, cx| {
            let limits = reading.await;
            let _ = runtime.update(cx, |runtime, cx| {
                runtime.usage.complete(id, limits);
                cx.notify();
            });
        });
        self.tasks.insert(key, task);
    }
    pub(super) fn stop(&mut self) {
        self.tasks.clear();
        self.reading.clear();
    }
}
