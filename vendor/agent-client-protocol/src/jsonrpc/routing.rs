//! Payload-independent lifecycle owners shared by typed and managed routing.
//! No codec, allocation policy, channel or callback is performed by lifecycle
//! transitions. Storage owners provide either original or prepaid backing.

pub(crate) trait PendingStorage {
    type Key;
    type Value;
    type Error;
    type Drain: Iterator<Item = Self::Value>;
    type Values<'a>: Iterator<Item = &'a Self::Value>
    where
        Self: 'a;
    fn prepare(&mut self) -> Result<(), Self::Error>;
    fn contains(&self, key: &Self::Key) -> bool;
    fn get(&self, key: &Self::Key) -> Option<&Self::Value>;
    fn insert_reserved(&mut self, key: Self::Key, value: Self::Value) -> Option<Self::Value>;
    fn remove(&mut self, key: &Self::Key) -> Option<Self::Value>;
    fn drain(&mut self) -> Self::Drain;
    fn values(&self) -> Self::Values<'_>;
}
/// Owns close-before-drain, admission and response correlation together.
pub(crate) struct PendingRegistry<S: PendingStorage> {
    storage: S,
    closed: bool,
}
impl<S: PendingStorage + Default> Default for PendingRegistry<S> {
    fn default() -> Self {
        Self::new(S::default())
    }
}
impl<S: PendingStorage> PendingRegistry<S> {
    pub(crate) fn new(storage: S) -> Self {
        Self {
            storage,
            closed: false,
        }
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }
    pub(crate) fn prepare(&mut self) -> Result<bool, S::Error> {
        if self.closed {
            return Ok(false);
        }
        self.storage.prepare()?;
        Ok(true)
    }
    pub(crate) fn subscribe_reserved(
        &mut self,
        key: S::Key,
        value: S::Value,
    ) -> Result<Option<S::Value>, S::Value> {
        if self.closed {
            Err(value)
        } else {
            Ok(self.storage.insert_reserved(key, value))
        }
    }
    pub(crate) fn contains(&self, key: &S::Key) -> bool {
        self.storage.contains(key)
    }
    pub(crate) fn get(&self, key: &S::Key) -> Option<&S::Value> { self.storage.get(key) }
    pub(crate) fn remove(&mut self, key: &S::Key) -> Option<S::Value> {
        self.storage.remove(key)
    }
    pub(crate) fn values(&self) -> S::Values<'_> {
        self.storage.values()
    }
    pub(crate) fn close(&mut self) -> S::Drain {
        self.closed = true;
        self.storage.drain()
    }
}
/// Original unmanaged HashMap storage; generic routing does not alter its policy.
pub(crate) struct StandardPending<K, V>(std::collections::HashMap<K, V>);
impl<K, V> Default for StandardPending<K, V> {
    fn default() -> Self {
        Self(std::collections::HashMap::new())
    }
}
impl<K: Eq + std::hash::Hash, V> PendingStorage for StandardPending<K, V> {
    type Key = K;
    type Value = V;
    type Error = std::convert::Infallible;
    type Drain = std::collections::hash_map::IntoValues<K, V>;
    type Values<'a>
        = std::collections::hash_map::Values<'a, K, V>
    where
        Self: 'a;
    fn prepare(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn contains(&self, key: &K) -> bool {
        self.0.contains_key(key)
    }
    fn get(&self, key: &K) -> Option<&V> { self.0.get(key) }
    fn insert_reserved(&mut self, key: K, value: V) -> Option<V> {
        self.0.insert(key, value)
    }
    fn remove(&mut self, key: &K) -> Option<V> {
        self.0.remove(key)
    }
    fn drain(&mut self) -> Self::Drain {
        std::mem::take(&mut self.0).into_values()
    }
    fn values(&self) -> Self::Values<'_> {
        self.0.values()
    }
}

pub(crate) struct BatchLifecycle {
    remaining: usize,
    dispatch_complete: bool,
    emitted: bool,
}
impl BatchLifecycle {
    pub(crate) fn new(remaining: usize) -> Self {
        Self {
            remaining,
            dispatch_complete: false,
            emitted: false,
        }
    }
    pub(crate) fn is_dispatched(&self) -> bool {
        self.dispatch_complete
    }
    pub(crate) fn is_emitted(&self) -> bool {
        self.emitted
    }
    pub(crate) fn finish_dispatch(&mut self) -> bool {
        if self.dispatch_complete {
            return false;
        }
        self.dispatch_complete = true;
        true
    }
    pub(crate) fn complete_slot(&mut self) {
        self.remaining -= 1;
    }
    pub(crate) fn take_completed(&mut self) -> bool {
        if !self.dispatch_complete || self.remaining != 0 || self.emitted {
            return false;
        }
        self.emitted = true;
        true
    }
}

/// Original lazy cancellation state, independent of marker/waiter allocation.
#[derive(Debug)]
pub(crate) enum CancellationState<M> {
    Armed,
    Cancelled,
    Marker(M),
}
impl<M: Clone> CancellationState<M> {
    pub(crate) fn cancel(&mut self) -> Option<M> {
        match self {
            Self::Armed => {
                *self = Self::Cancelled;
                None
            }
            Self::Cancelled => None,
            Self::Marker(marker) => Some(marker.clone()),
        }
    }
    pub(crate) fn marker<E>(&mut self, create: impl FnOnce(bool) -> Result<M, E>) -> Result<M, E> {
        match self {
            Self::Marker(marker) => Ok(marker.clone()),
            _ => {
                let marker = create(matches!(self, Self::Cancelled))?;
                *self = Self::Marker(marker.clone());
                Ok(marker)
            }
        }
    }
}
