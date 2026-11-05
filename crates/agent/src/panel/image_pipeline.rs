//! Bounded previews retain immutable sources, so pointer identities cannot be reused.
use crate::ImageDeps;
use gpui_kit::{Image, Task};
use nocterm_ai::{
    images::{ImageInspector, MAX_PREVIEW_BYTES},
    model::content::ImageData,
};
use std::{cell::Cell, collections::HashMap, sync::Arc};
pub(super) type ImageKey = (usize, usize, bool);
const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_PREVIEW_RESIDENT: usize = MAX_PREVIEW_BYTES + 640 * 640 * 8;
const MAX_LOADING: usize = 4;

#[derive(Clone)]
pub(super) enum PreviewSource {
    Encoded(ImageData),
    Original(Arc<[u8]>),
}
impl PreviewSource {
    pub(super) fn retained_bytes(&self) -> usize {
        match self {
            Self::Encoded(data) => data.retained_bytes(),
            Self::Original(data) => data.len().saturating_add(
                2 * std::mem::size_of::<usize>() + std::mem::align_of::<usize>() - 1,
            ),
        }
    }
    pub(super) fn key(&self) -> ImageKey {
        match self {
            Self::Encoded(data) => (data.as_ptr() as usize, data.len(), true),
            Self::Original(data) => (data.as_ptr() as usize, data.len(), false),
        }
    }
    pub(super) fn decode(&self) -> Result<Arc<[u8]>, super::entries::PreviewError> {
        match self {
            Self::Original(data) => Ok(data.clone()),
            Self::Encoded(data) => {
                use base64::Engine as _;
                if data.len() > nocterm_ai::images::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                    return Err(nocterm_ai::images::ImageCodecError::SourceSize.into());
                }
                Ok(base64::engine::general_purpose::STANDARD
                    .decode(data)?
                    .into())
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PreviewTicket {
    generation: u64,
    request: u64,
    key: ImageKey,
}
#[derive(Clone)]
pub(super) struct CompletedPreview {
    pub(super) image: Arc<Image>,
    pub(super) resident_bytes: usize,
}
pub(super) enum PreviewState {
    Loading,
    Ready(Arc<Image>),
    Failed,
}
struct PreviewEntry {
    ticket: PreviewTicket,
    state: PreviewState,
    source: PreviewSource,
    last_used: Cell<u64>,
    task: Option<Task<()>>,
    charge: usize,
}
struct PreviewCache {
    entries: HashMap<ImageKey, PreviewEntry>,
    generation: u64,
    next_request: u64,
    clock: Cell<u64>,
}
pub(super) struct ImagePipeline {
    cache: PreviewCache,
    deps: ImageDeps,
}
impl ImagePipeline {
    pub(super) fn new(deps: ImageDeps) -> Self {
        Self {
            cache: PreviewCache {
                entries: HashMap::new(),
                generation: 0,
                next_request: 0,
                clock: Cell::new(0),
            },
            deps,
        }
    }
    pub(super) fn inspector(&self) -> Arc<dyn ImageInspector> {
        self.deps.inspector()
    }
    pub(super) fn image_deps(&self) -> ImageDeps {
        self.deps.clone()
    }
    pub(super) fn admit_work(&self) -> Option<crate::image_deps::ImageWorkPermit> {
        self.deps.admit_work()
    }
    pub(super) fn contains_key(&self, key: &ImageKey) -> bool {
        self.state(key).is_some()
    }
    pub(super) fn state(&self, key: &ImageKey) -> Option<&PreviewState> {
        let entry = self.cache.entries.get(key)?;
        let next = self.cache.clock.get().saturating_add(1);
        self.cache.clock.set(next);
        entry.last_used.set(next);
        Some(&entry.state)
    }
    pub(super) fn request(&mut self, source: PreviewSource) -> Option<PreviewTicket> {
        let key = source.key();
        if self.contains_key(&key) {
            return None;
        }
        let loading = self
            .cache
            .entries
            .values()
            .filter(|entry| matches!(entry.state, PreviewState::Loading))
            .count();
        if loading >= MAX_LOADING {
            return None;
        }
        let charge = source.retained_bytes().saturating_add(MAX_PREVIEW_RESIDENT);
        if charge > MAX_BYTES {
            return None;
        }
        while self.cache.entries.len() >= MAX_ENTRIES
            || self.charged_bytes().saturating_add(charge) > MAX_BYTES
        {
            let oldest = self
                .cache
                .entries
                .iter()
                .filter(|(_, entry)| !matches!(entry.state, PreviewState::Loading))
                .min_by_key(|(_, entry)| entry.last_used.get())
                .map(|(key, _)| *key)?;
            self.cache.entries.remove(&oldest);
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
        let last_used = self.cache.clock.get().saturating_add(1);
        self.cache.clock.set(last_used);
        self.cache.entries.insert(
            key,
            PreviewEntry {
                ticket,
                state: PreviewState::Loading,
                source,
                last_used: Cell::new(last_used),
                task: None,
                charge,
            },
        );
        Some(ticket)
    }
    fn charged_bytes(&self) -> usize {
        self.cache.entries.values().map(|entry| entry.charge).sum()
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
    pub(super) fn complete(
        &mut self,
        ticket: PreviewTicket,
        image: Option<CompletedPreview>,
    ) -> bool {
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
        entry.charge = entry
            .source
            .retained_bytes()
            .saturating_add(image.as_ref().map_or(0, |image| image.resident_bytes));
        entry.state = image.map_or(PreviewState::Failed, |image| {
            PreviewState::Ready(image.image)
        });
        entry.task = None;
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
