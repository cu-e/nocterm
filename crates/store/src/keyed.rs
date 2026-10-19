//! Coalesced independent-key publications driven by the common Store worker.
use crate::{
    Revision, StoreError,
    worker::{self, DiskOperation, DiskWork, DriverControl, NormalOwner},
};
use futures::future::BoxFuture;
use gpui::{App, Context};
use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};
mod capture;
mod ownership;
pub use capture::{CapturedKeys, KeyedOutcome};
pub use ownership::{CloneValues, KeyedCapture, KeyedValueOwnership};
pub type KeyedWriter<K, V> =
    Arc<dyn Fn(K, V) -> BoxFuture<'static, Result<(), StoreError>> + Send + Sync>;
/// Retain registered-key metadata, or release a clean closed key after its hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyedRetention {
    Retain,
    ForgetClean,
}
type CompletionHook<K, Tag> =
    Box<dyn Fn(&KeyedStamp<K, Tag>, Result<(), StoreError>, &mut App) -> KeyedRetention>;

/// The exact key, owner tag and admission generation physically published.
#[derive(Clone, Debug)]
pub struct KeyedStamp<K, Tag> {
    key: K,
    tag: Tag,
    generation: Revision,
}
impl<K, Tag> KeyedStamp<K, Tag> {
    pub fn key(&self) -> &K {
        &self.key
    }
    pub fn tag(&self) -> &Tag {
        &self.tag
    }
    pub fn generation(&self) -> Revision {
        self.generation
    }
}
#[derive(Clone)]
struct Candidate<K, V, Tag> {
    stamp: KeyedStamp<K, Tag>,
    value: V,
}
struct Entry<K, V, Tag> {
    latest: Option<Candidate<K, V, Tag>>,
    queued: bool,
    waiting: Option<Revision>,
    blocked: Option<Revision>,
    error: Option<StoreError>,
    succeeded: Option<KeyedStamp<K, Tag>>,
}
impl<K, V, Tag> Default for Entry<K, V, Tag> {
    fn default() -> Self {
        Self {
            latest: None,
            queued: false,
            waiting: None,
            blocked: None,
            error: None,
            succeeded: None,
        }
    }
}
type DiskStamps<K, Tag> = Arc<Mutex<HashMap<K, KeyedStamp<K, Tag>>>>;

/// One worker and at most `capacity` ready-key tokens; accepted dirty keys are retained.
/// No full committed values are retained after success and no per-edit receipt is allocated.
pub struct KeyedPersisted<K, V, Tag, P: KeyedValueOwnership<V> = CloneValues> {
    entries: HashMap<K, Entry<K, V, Tag>>,
    ready: VecDeque<K>,
    active: Option<KeyedStamp<K, Tag>>,
    writer: KeyedWriter<K, P::Write>,
    ownership: Arc<P>,
    control: DriverControl,
    sequence: Revision,
    closing: bool,
    hook: Option<CompletionHook<K, Tag>>,
    disk: DiskStamps<K, Tag>,
    capacity: NonZeroUsize,
}
impl<K: Clone + Eq + Hash + Send + 'static, V: Clone + Send + 'static, Tag: Clone + Send + 'static>
    KeyedPersisted<K, V, Tag>
{
    pub fn new(writer: KeyedWriter<K, V>, capacity: NonZeroUsize) -> Self {
        Self::with_owned_values(writer, capacity, CloneValues)
    }
}
impl<
    K: Clone + Eq + Hash + Send + 'static,
    V: Send + 'static,
    Tag: Clone + Send + 'static,
    P: KeyedValueOwnership<V>,
> KeyedPersisted<K, V, Tag, P>
{
    pub fn with_owned_values(
        writer: KeyedWriter<K, P::Write>,
        capacity: NonZeroUsize,
        ownership: P,
    ) -> Self {
        Self {
            entries: HashMap::new(),
            ready: VecDeque::new(),
            active: None,
            writer,
            ownership: Arc::new(ownership),
            control: DriverControl::default(),
            sequence: 0,
            closing: false,
            hook: None,
            disk: Arc::new(Mutex::new(HashMap::new())),
            capacity,
        }
    }
    /// Runs synchronously before any owner-visible acknowledgement; may not access this entity.
    pub fn with_completion_hook(
        mut self,
        hook: impl Fn(&KeyedStamp<K, Tag>, Result<(), StoreError>, &mut App) -> KeyedRetention + 'static,
    ) -> Self {
        self.hook = Some(Box::new(hook));
        self
    }
    pub fn update_latest(
        &mut self,
        key: K,
        mut value: V,
        tag: Tag,
        cx: &mut Context<Self>,
    ) -> Result<Revision, StoreError> {
        let ownership = self.ownership.clone();
        self.update_latest_owned(
            key,
            tag,
            move |previous| {
                ownership.accept(&mut value, previous)?;
                Ok(value)
            },
            cx,
        )
    }
    /// Synchronous intake can transfer a unique original capability from the
    /// previous value. On error the factory must preserve that previous value.
    /// No factory or borrowed source is retained by the worker.
    pub fn update_latest_owned(
        &mut self,
        key: K,
        tag: Tag,
        build: impl FnOnce(Option<&mut V>) -> Result<V, StoreError>,
        cx: &mut Context<Self>,
    ) -> Result<Revision, StoreError> {
        if self.closing {
            return Err(StoreError::Closing);
        }
        let value = build(
            self.entries
                .get_mut(&key)
                .and_then(|entry| entry.latest.as_mut())
                .map(|candidate| &mut candidate.value),
        )?;
        let entry = self.entries.entry(key.clone()).or_default();
        self.sequence += 1;
        let generation = self.sequence;
        entry.latest = Some(Candidate {
            stamp: KeyedStamp {
                key: key.clone(),
                tag,
                generation,
            },
            value,
        });
        entry.waiting.get_or_insert(generation);
        entry.blocked = None;
        entry.error = None;
        self.enqueue(&key);
        if !self.control.is_running() {
            worker::start(self, cx)
        }
        cx.notify();
        Ok(generation)
    }
    pub fn retry(&mut self, key: &K, cx: &mut Context<Self>) -> Result<(), StoreError> {
        if self.closing {
            return Err(StoreError::Closing);
        }
        if let Some(entry) = self.entries.get_mut(key)
            && entry.latest.is_some()
        {
            entry.blocked = None;
            entry.error = None;
            entry.waiting.get_or_insert(self.sequence);
            self.enqueue(key);
            if !self.control.is_running() {
                worker::start(self, cx)
            }
        }
        Ok(())
    }
    pub fn error(&self, key: &K) -> Option<&StoreError> {
        self.entries.get(key)?.error.as_ref()
    }
    pub fn last_success(&self, key: &K) -> Option<&KeyedStamp<K, Tag>> {
        self.entries.get(key)?.succeeded.as_ref()
    }
    pub fn is_dirty(&self, key: &K) -> bool {
        self.entries
            .get(key)
            .is_some_and(|entry| entry.latest.is_some())
    }
    /// Remove a registered clean key; false also means the key was absent.
    pub fn forget_clean(&mut self, key: &K) -> bool {
        if self.is_dirty(key)
            || self
                .active
                .as_ref()
                .is_some_and(|candidate| &candidate.key == key)
        {
            return false;
        }
        let removed = self.entries.remove(key).is_some();
        self.disk
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        removed
    }
    fn eligible(entry: &Entry<K, V, Tag>) -> bool {
        entry.waiting.is_some()
            && entry
                .latest
                .as_ref()
                .is_some_and(|latest| entry.blocked != Some(latest.stamp.generation))
    }
    fn enqueue(&mut self, key: &K) {
        if self.ready.len() >= self.capacity.get() {
            return;
        }
        if let Some(entry) = self.entries.get_mut(key)
            && !entry.queued
            && Self::eligible(entry)
        {
            entry.queued = true;
            self.ready.push_back(key.clone())
        }
    }
    fn refill(&mut self) {
        let remaining = self.capacity.get().saturating_sub(self.ready.len());
        if remaining == 0 {
            return;
        }
        let mut waiting = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.queued && Self::eligible(entry))
            .map(|(key, entry)| (entry.waiting.unwrap_or(0), key.clone()))
            .collect::<Vec<_>>();
        waiting.sort_by_key(|(sequence, _)| *sequence);
        for (_, key) in waiting.into_iter().take(remaining) {
            self.enqueue(&key)
        }
    }
    fn operation(
        &self,
        stamp: KeyedStamp<K, Tag>,
        capture: Result<KeyedCapture<P::Write, P::Guard>, StoreError>,
    ) -> DiskOperation<()> {
        let writer = self.writer.clone();
        let disk = self.disk.clone();
        let key = stamp.key.clone();
        DiskOperation::new(
            move || match capture {
                Ok(capture) => capture.run(key, writer),
                Err(error) => Box::pin(async move { Err(error) }),
            },
            move |result| {
                if result.is_ok() {
                    disk.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(stamp.key.clone(), stamp);
                }
            },
        )
    }
    pub fn capture(&mut self) -> CapturedKeys<K, V, Tag, P> {
        self.closing = true;
        self.control.freeze();
        let mut candidates = self
            .entries
            .values_mut()
            .filter_map(|entry| entry.latest.take())
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.stamp.generation);
        self.ready.clear();
        self.active.take();
        CapturedKeys::new(
            candidates,
            self.writer.clone(),
            self.control.gate(),
            self.disk.clone(),
            self.ownership.clone(),
        )
    }
}
impl<
    K: Clone + Eq + Hash + Send + 'static,
    V: Send + 'static,
    Tag: Clone + Send + 'static,
    P: KeyedValueOwnership<V>,
> NormalOwner for KeyedPersisted<K, V, Tag, P>
{
    type Physical = ();
    type Tag = KeyedStamp<K, Tag>;
    fn control(&mut self) -> &mut DriverControl {
        &mut self.control
    }
    fn take_operation(&mut self, _cx: &mut Context<Self>) -> Option<DiskWork<(), Self::Tag>> {
        self.refill();
        while let Some(key) = self.ready.pop_front() {
            let Some(entry) = self.entries.get_mut(&key) else {
                continue;
            };
            entry.queued = false;
            if !Self::eligible(entry) {
                continue;
            }
            let Some(candidate) = entry.latest.as_ref() else {
                continue;
            };
            let stamp = candidate.stamp.clone();
            let capture = self.ownership.capture(&candidate.value);
            entry.waiting = None;
            self.active = Some(stamp.clone());
            self.refill();
            return Some(DiskWork::new(
                self.operation(stamp.clone(), capture),
                stamp,
                None,
            ));
        }
        None
    }
    fn apply_outcome(
        &mut self,
        stamp: Self::Tag,
        result: Result<(), StoreError>,
        cx: &mut Context<Self>,
    ) {
        self.active = None;
        if let Some(entry) = self.entries.get_mut(&stamp.key) {
            if result.is_ok() {
                entry.succeeded = Some(stamp.clone())
            }
            if entry
                .latest
                .as_ref()
                .is_some_and(|latest| latest.stamp.generation == stamp.generation)
            {
                match &result {
                    Ok(()) => {
                        entry.latest = None;
                        entry.error = None;
                        entry.waiting = None;
                    }
                    Err(error) => {
                        entry.blocked = Some(stamp.generation);
                        entry.error = Some(error.clone());
                    }
                }
            }
        }
        let retention = self
            .hook
            .as_ref()
            .map_or(KeyedRetention::Retain, |hook| hook(&stamp, result, cx));
        if retention == KeyedRetention::ForgetClean {
            self.forget_clean(&stamp.key);
        }
        self.refill();
        cx.notify();
    }
}
#[cfg(test)]
mod tests;
