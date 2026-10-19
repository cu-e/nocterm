//! Immutable independent-key policy for the one Store captured runner.
use super::{Candidate, DiskStamps, KeyedStamp, KeyedWriter};
use crate::{
    StoreError,
    worker::{
        DiskOperation,
        captured::{self, CapturedPolicy},
    },
};
use futures::future::BoxFuture;
use nocterm_core::persist::WriteGate;
use std::{collections::VecDeque, hash::Hash};

pub struct KeyedOutcome<K, Tag> {
    stamp: KeyedStamp<K, Tag>,
    result: Result<(), StoreError>,
    committed: Option<KeyedStamp<K, Tag>>,
}
impl<K, Tag> KeyedOutcome<K, Tag> {
    pub fn stamp(&self) -> &KeyedStamp<K, Tag> {
        &self.stamp
    }
    pub fn committed(&self) -> Option<&KeyedStamp<K, Tag>> {
        self.committed.as_ref()
    }
    pub fn result(&self) -> Result<(), StoreError> {
        self.result.clone()
    }
}
pub struct CapturedKeys<K, V, Tag> {
    pending: VecDeque<Candidate<K, V, Tag>>,
    writer: KeyedWriter<K, V>,
    gate: WriteGate,
    disk: DiskStamps<K, Tag>,
    active: Option<KeyedStamp<K, Tag>>,
    outcomes: Vec<KeyedOutcome<K, Tag>>,
}
impl<K: Clone + Eq + Hash + Send + 'static, V: Clone + Send + 'static, Tag: Clone + Send + 'static>
    CapturedKeys<K, V, Tag>
{
    pub(super) fn new(
        candidates: Vec<Candidate<K, V, Tag>>,
        writer: KeyedWriter<K, V>,
        gate: WriteGate,
        disk: DiskStamps<K, Tag>,
    ) -> Self {
        Self {
            pending: candidates.into(),
            writer,
            gate,
            disk,
            active: None,
            outcomes: Vec::new(),
        }
    }
    pub fn run(self) -> BoxFuture<'static, Vec<KeyedOutcome<K, Tag>>> {
        captured::run(self)
    }
}
impl<K: Clone + Eq + Hash + Send + 'static, V: Clone + Send + 'static, Tag: Clone + Send + 'static>
    CapturedPolicy for CapturedKeys<K, V, Tag>
{
    type Physical = ();
    type Output = Vec<KeyedOutcome<K, Tag>>;
    fn gate(&self) -> WriteGate {
        self.gate.clone()
    }
    fn joined(&mut self) {}
    fn next_operation(&mut self) -> BoxFuture<'_, Option<DiskOperation<()>>> {
        Box::pin(async move {
            while let Some(candidate) = self.pending.pop_front() {
                let committed = self
                    .disk
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&candidate.stamp.key)
                    .is_some_and(|stamp| stamp.generation == candidate.stamp.generation);
                if committed {
                    self.outcomes.push(KeyedOutcome {
                        stamp: candidate.stamp,
                        result: Ok(()),
                        committed: None,
                    });
                    continue;
                }
                self.active = Some(candidate.stamp.clone());
                let writer = self.writer.clone();
                let disk = self.disk.clone();
                let stamp = candidate.stamp.clone();
                return Some(DiskOperation::new(
                    move || writer(candidate.stamp.key, candidate.value),
                    move |result| {
                        if result.is_ok() {
                            disk.lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(stamp.key.clone(), stamp);
                        }
                    },
                ));
            }
            None
        })
    }
    fn apply_outcome(&mut self, result: Result<(), StoreError>) {
        if let Some(stamp) = self.active.take() {
            self.outcomes.push(KeyedOutcome {
                stamp,
                result,
                committed: None,
            })
        }
    }
    fn finish(mut self) -> Self::Output {
        let disk = self.disk.lock().unwrap_or_else(|error| error.into_inner());
        for outcome in &mut self.outcomes {
            outcome.committed = disk.get(outcome.stamp.key()).cloned();
        }
        self.outcomes
    }
}
