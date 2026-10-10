//! Retained usage projections and cancellable refresh ownership.
use super::Runtime;
use gpui::{Context, Task};
use nocterm_ai::usage::{Limits, RateLimit};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
#[derive(Default)]
pub(super) struct UsageLimits {
    limits: HashMap<String, Limits>,
    reading: HashSet<String>,
    tasks: HashMap<String, Task<()>>,
}
impl UsageLimits {
    pub(super) fn get(&self, id: &str) -> Option<&Limits> {
        self.limits.get(id).filter(|limits| !limits.is_empty())
    }
    pub(super) fn merge(&mut self, id: String, limit: RateLimit) {
        self.limits.entry(id).or_default().merge(limit)
    }
    fn complete(&mut self, id: String, limits: Option<Vec<RateLimit>>) {
        self.reading.remove(&id);
        self.tasks.remove(&id);
        if let Some(limits) = limits {
            self.limits.entry(id).or_default().replace(limits)
        }
    }
    pub(super) fn refresh(&mut self, id: String, home: PathBuf, cx: &mut Context<Runtime>) {
        if !self.reading.insert(id.clone()) {
            return;
        }
        let reading = cx
            .background_executor()
            .spawn(async move { nocterm_ai::usage::read_codex_limits(&home) });
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
