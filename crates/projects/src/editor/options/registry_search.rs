//! Bounded picker-local registry work and cache; source identity is part of every key.
use gpui_kit::Task;
use nocterm_containers::Engine;
use nocterm_project_providers::{ImageSearchError, RegistryImage};
use nocterm_session::local_fs::CancelFlag;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct SearchKey {
    source: String,
    engine: Engine,
    query: String,
}
impl SearchKey {
    pub(super) fn new(source: String, engine: Engine, query: &str) -> Self {
        Self {
            source,
            engine,
            query: query.trim().to_ascii_lowercase(),
        }
    }
}
struct CachedImages {
    key: SearchKey,
    fetched: Instant,
    images: Vec<RegistryImage>,
}
pub(super) enum SearchStatus {
    Idle,
    Pending,
    Complete,
    Failed(String),
}
pub(super) struct RegistrySearchState {
    generation: u64,
    cancel: CancelFlag,
    task: Option<Task<()>>,
    status: SearchStatus,
    results: Vec<RegistryImage>,
    cooldown: Option<Instant>,
    cache: VecDeque<CachedImages>,
    selected: Option<String>,
}
impl Default for RegistrySearchState {
    fn default() -> Self {
        Self {
            generation: 0,
            cancel: CancelFlag::default(),
            task: None,
            status: SearchStatus::Idle,
            results: vec![],
            cooldown: None,
            cache: VecDeque::new(),
            selected: None,
        }
    }
}
impl RegistrySearchState {
    pub(super) fn invalidate(&mut self, clear: bool) {
        self.generation = self.generation.wrapping_add(1);
        self.cancel.cancel();
        self.task = None;
        self.cancel = CancelFlag::default();
        if clear {
            self.results.clear();
            self.status = SearchStatus::Idle;
        }
    }
    pub(super) fn select(&mut self, reference: String) {
        self.invalidate(false);
        self.selected = Some(reference);
        self.status = SearchStatus::Complete;
    }
    pub(super) fn was_selected(&mut self, query: &str) -> bool {
        if self.selected.as_deref() == Some(query) {
            return true;
        }
        self.selected = None;
        false
    }
    pub(super) fn prepare(&mut self, key: &SearchKey, now: Instant) -> Option<(u64, CancelFlag)> {
        self.invalidate(true);
        self.cache
            .retain(|entry| now.duration_since(entry.fetched) < Duration::from_secs(90));
        if let Some(entry) = self.cache.iter().find(|entry| &entry.key == key) {
            self.results = entry.images.clone();
            self.status = SearchStatus::Complete;
            return None;
        }
        if self.cooldown.is_some_and(|until| now < until) {
            self.status = SearchStatus::Failed(
                "Registry search is rate limited. Wait a minute, or enter a full image reference."
                    .into(),
            );
            return None;
        }
        self.status = SearchStatus::Pending;
        Some((self.generation, self.cancel.clone()))
    }
    pub(super) fn set_task(&mut self, task: Task<()>) {
        self.task = Some(task);
    }
    pub(super) fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }
    pub(super) fn complete(
        &mut self,
        generation: u64,
        key: SearchKey,
        result: Result<Vec<RegistryImage>, ImageSearchError>,
        now: Instant,
    ) {
        if !self.is_current(generation) {
            return;
        }
        match result {
            Ok(images) => {
                self.results = images.into_iter().take(25).collect();
                self.cache.retain(|entry| entry.key != key);
                self.cache.push_back(CachedImages {
                    key,
                    fetched: now,
                    images: self.results.clone(),
                });
                while self.cache.len() > 16 {
                    self.cache.pop_front();
                }
                self.status = SearchStatus::Complete;
            }
            Err(error) => {
                if let ImageSearchError::RateLimited { cooldown } = &error {
                    self.cooldown = Some(now + (*cooldown).max(Duration::from_secs(60)));
                }
                self.status = SearchStatus::Failed(format!(
                    "{error}. Enter a full reference, or edit the search to try again."
                ));
            }
        }
    }
    pub(super) fn results(&self) -> &[RegistryImage] {
        &self.results
    }
    pub(super) fn status(&self) -> &SearchStatus {
        &self.status
    }
}
impl Drop for RegistrySearchState {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
