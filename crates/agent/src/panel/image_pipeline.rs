//! Preview requests retain cache generation and identity across background delivery.
use crate::ImageDeps;
use gpui_kit::{Image, Task};
use nocterm_ai::images::{ImageInspector, ImageThumbnailer};
use std::{collections::HashMap, sync::Arc};
pub(super) type ImageKey = (usize, usize, bool);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PreviewTicket {
    generation: u64,
    request: u64,
    key: ImageKey,
}
pub(super) enum PreviewState {
    Loading,
    Ready(Arc<Image>),
    Failed,
}
struct PreviewEntry {
    ticket: PreviewTicket,
    state: PreviewState,
    task: Option<Task<()>>,
}
struct PreviewCache {
    entries: HashMap<ImageKey, PreviewEntry>,
    generation: u64,
    next_request: u64,
}
pub(super) struct ImagePipeline {
    cache: PreviewCache,
    inspector: Arc<dyn ImageInspector>,
    thumbnailer: Arc<dyn ImageThumbnailer>,
}
impl ImagePipeline {
    pub(super) fn new(deps: ImageDeps) -> Self {
        Self {
            cache: PreviewCache {
                entries: HashMap::new(),
                generation: 0,
                next_request: 0,
            },
            inspector: deps.inspector(),
            thumbnailer: deps.thumbnailer(),
        }
    }
    pub(super) fn inspector(&self) -> Arc<dyn ImageInspector> {
        self.inspector.clone()
    }
    pub(super) fn thumbnailer(&self) -> Arc<dyn ImageThumbnailer> {
        self.thumbnailer.clone()
    }
    pub(super) fn contains_key(&self, key: &ImageKey) -> bool {
        self.cache.entries.contains_key(key)
    }
    pub(super) fn state(&self, key: &ImageKey) -> Option<&PreviewState> {
        self.cache.entries.get(key).map(|entry| &entry.state)
    }
    pub(super) fn request(&mut self, key: ImageKey) -> Option<PreviewTicket> {
        if self.contains_key(&key) {
            return None;
        }
        self.cache.next_request = self
            .cache
            .next_request
            .checked_add(1)
            .expect("preview request space");
        let ticket = PreviewTicket {
            generation: self.cache.generation,
            request: self.cache.next_request,
            key,
        };
        self.cache.entries.insert(
            key,
            PreviewEntry {
                ticket,
                state: PreviewState::Loading,
                task: None,
            },
        );
        Some(ticket)
    }
    pub(super) fn install_task(&mut self, ticket: PreviewTicket, task: Task<()>) {
        if let Some(entry) = self
            .cache
            .entries
            .get_mut(&ticket.key)
            .filter(|entry| entry.ticket == ticket)
        {
            entry.task = Some(task);
        }
    }
    pub(super) fn complete(&mut self, ticket: PreviewTicket, image: Option<Arc<Image>>) -> bool {
        let Some(entry) = self
            .cache
            .entries
            .get_mut(&ticket.key)
            .filter(|entry| entry.ticket == ticket)
        else {
            return false;
        };
        if !matches!(entry.state, PreviewState::Loading) {
            return false;
        }
        entry.state = image.map_or(PreviewState::Failed, PreviewState::Ready);
        true
    }
    pub(super) fn clear(&mut self) {
        self.cache.generation = self
            .cache
            .generation
            .checked_add(1)
            .expect("preview generation space");
        self.cache.entries.clear();
    }
}
#[cfg(test)]
mod tests;
