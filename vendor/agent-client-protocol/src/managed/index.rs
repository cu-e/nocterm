//! Fallible lookup storage with the same measured buckets as RPC correlation.
use super::{Budget, ManagedError, map::Map};
use std::{borrow::Borrow, hash::Hash};

/// A prepaid index. Keys and values remain owned by the caller on refusal.
/// Replacing a value retains its original key and requires no bucket growth.
pub struct Index<K, V, B: Budget> {
    entries: Map<K, V, B>,
}
impl<K: Eq + Hash, V, B: Budget> Index<K, V, B> {
    /// Start an empty index without allocating buckets.
    pub fn new(budget: B) -> Self {
        Self {
            entries: Map::new(budget),
        }
    }
    /// Borrow the value associated with an equal key.
    pub fn get<Q: Eq + Hash + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.entries.get(key)
    }
    /// Mutate a present value without changing its key or bucket storage.
    pub fn get_mut<Q: Eq + Hash + ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        self.entries.get_mut(key)
    }
    /// Remove a value using a borrowed key with the same Hash/Eq contract.
    pub fn remove<Q: Eq + Hash + ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        self.entries.remove(key)
    }
    /// Insert after prepaying growth, or replace an equal key without growth.
    /// On failure, return the untouched key and value with the typed refusal.
    pub fn insert(
        &mut self,
        key: K,
        value: V,
    ) -> Result<Option<V>, (ManagedError<B::Error>, K, V)> {
        if let Some(old) = self.entries.get_mut(&key) {
            return Ok(Some(std::mem::replace(old, value)));
        }
        if let Err(error) = self.entries.reserve() {
            return Err((error, key, value));
        }
        Ok(self.entries.insert_reserved(key, value))
    }
}
