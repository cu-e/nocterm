//! Exact-layout open addressing; no hidden standard HashMap bucket allocation.
use super::{Budget, Buffer, ManagedError};
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
};

enum Bucket<K, V> {
    Empty,
    Deleted,
    Occupied(K, V),
}
pub(super) struct Map<K, V, B: Budget> {
    buckets: Buffer<Bucket<K, V>, B>,
    len: usize,
    deleted: usize,
    budget: B,
}
impl<K: Eq + Hash, V, B: Budget> Map<K, V, B> {
    pub(super) fn new(budget: B) -> Self {
        Self {
            buckets: Buffer::new(budget.clone()),
            len: 0,
            deleted: 0,
            budget,
        }
    }
    fn hash<Q: Hash + ?Sized>(key: &Q) -> usize {
        let mut hasher = rustc_hash::FxHasher::default();
        key.hash(&mut hasher);
        hasher.finish() as usize
    }
    fn locate<Q: Eq + Hash + ?Sized>(&self, key: &Q) -> Result<usize, usize>
    where
        K: Borrow<Q>,
    {
        if self.buckets.is_empty() {
            return Err(0);
        }
        let mask = self.buckets.len() - 1;
        let start = Self::hash(key) & mask;
        let mut deleted = None;
        for offset in 0..self.buckets.len() {
            let slot = (start + offset) & mask;
            match &self.buckets.as_slice()[slot] {
                Bucket::Occupied(old, _) if old.borrow() == key => return Ok(slot),
                Bucket::Occupied(_, _) => {}
                Bucket::Deleted => {
                    deleted.get_or_insert(slot);
                }
                Bucket::Empty => return Err(deleted.unwrap_or(slot)),
            }
        }
        Err(deleted.unwrap_or(0))
    }
    pub(super) fn reserve(&mut self) -> Result<(), ManagedError<B::Error>> {
        let occupied = self
            .len
            .checked_add(self.deleted)
            .and_then(|v| v.checked_add(1))
            .ok_or(ManagedError::CapacityOverflow)?;
        if occupied <= self.buckets.len() / 2 {
            return Ok(());
        }
        let capacity = self
            .len
            .checked_add(1)
            .and_then(|v| v.checked_mul(2))
            .and_then(usize::checked_next_power_of_two)
            .ok_or(ManagedError::CapacityOverflow)?;
        let mut replacement = Self::new(self.budget.clone());
        replacement.buckets.reserve(capacity)?;
        for _ in 0..capacity {
            replacement
                .buckets
                .push(Bucket::Empty)
                .map_err(|(error, _)| error)?;
        }
        for slot in self.buckets.as_mut_slice() {
            if let Bucket::Occupied(key, value) = std::mem::replace(slot, Bucket::Empty) {
                replacement.insert_reserved(key, value);
            }
        }
        *self = replacement;
        Ok(())
    }
    pub(super) fn insert_reserved(&mut self, key: K, value: V) -> Option<V> {
        let slot = match self.locate(&key) {
            Ok(slot) | Err(slot) => slot,
        };
        match std::mem::replace(
            &mut self.buckets.as_mut_slice()[slot],
            Bucket::Occupied(key, value),
        ) {
            Bucket::Occupied(_, old) => Some(old),
            Bucket::Deleted => {
                self.deleted -= 1;
                self.len += 1;
                None
            }
            Bucket::Empty => {
                self.len += 1;
                None
            }
        }
    }
    pub(super) fn get_mut<Q: Eq + Hash + ?Sized>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
    {
        let slot = self.locate(key).ok()?;
        match &mut self.buckets.as_mut_slice()[slot] {
            Bucket::Occupied(_, value) => Some(value),
            _ => None,
        }
    }
    pub(super) fn get<Q: Eq + Hash + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        let slot = self.locate(key).ok()?;
        match &self.buckets.as_slice()[slot] {
            Bucket::Occupied(_, value) => Some(value),
            _ => None,
        }
    }
    pub(super) fn remove<Q: Eq + Hash + ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
    {
        let slot = self.locate(key).ok()?;
        if let Bucket::Occupied(_, value) =
            std::mem::replace(&mut self.buckets.as_mut_slice()[slot], Bucket::Deleted)
        {
            self.len -= 1;
            self.deleted += 1;
            Some(value)
        } else {
            None
        }
    }
    pub(super) fn values(&self) -> MapValues<'_, K, V> {
        MapValues {
            buckets: self.buckets.as_slice().iter(),
        }
    }
    pub(super) fn drain(&mut self) -> Drain<K, V, B> {
        self.len = 0;
        self.deleted = 0;
        Drain {
            buckets: std::mem::replace(&mut self.buckets, Buffer::new(self.budget.clone())),
            index: 0,
        }
    }
}
pub(super) struct Drain<K, V, B: Budget> {
    buckets: Buffer<Bucket<K, V>, B>,
    index: usize,
}
impl<K, V, B: Budget> Iterator for Drain<K, V, B> {
    type Item = V;
    fn next(&mut self) -> Option<V> {
        while self.index < self.buckets.len() {
            let index = self.index;
            self.index += 1;
            if let Bucket::Occupied(_, value) =
                std::mem::replace(&mut self.buckets.as_mut_slice()[index], Bucket::Empty)
            {
                return Some(value);
            }
        }
        None
    }
}

pub(crate) struct MapValues<'a, K, V> {
    buckets: std::slice::Iter<'a, Bucket<K, V>>,
}
impl<'a, K, V> Iterator for MapValues<'a, K, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<Self::Item> {
        for bucket in self.buckets.by_ref() {
            if let Bucket::Occupied(_, value) = bucket {
                return Some(value);
            }
        }
        None
    }
}
impl<K: Eq + std::hash::Hash, V, B: Budget> crate::jsonrpc::routing::PendingStorage
    for Map<K, V, B>
{
    type Key = K;
    type Value = V;
    type Error = ManagedError<B::Error>;
    type Drain = Drain<K, V, B>;
    type Values<'a>
        = MapValues<'a, K, V>
    where
        Self: 'a;
    fn prepare(&mut self) -> Result<(), Self::Error> {
        Map::reserve(self)
    }
    fn get(&self, key: &K) -> Option<&V> { Map::get(self,key) }
    fn contains(&self, key: &K) -> bool {
        self.get(key).is_some()
    }
    fn insert_reserved(&mut self, key: K, value: V) -> Option<V> {
        Map::insert_reserved(self, key, value)
    }
    fn remove(&mut self, key: &K) -> Option<V> {
        Map::remove(self, key)
    }
    fn drain(&mut self) -> Self::Drain {
        Map::drain(self)
    }
    fn values(&self) -> Self::Values<'_> {
        Map::values(self)
    }
}
