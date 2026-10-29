//! Per-capture source lifetimes for consumer tests; no authentication is performed.
use crate::{
    HubError,
    source_identity::{
        LocalAccountPartition, LocalAccountSource, LocalAccountSourceError,
        LocalAccountSourceFactory,
    },
};
use std::sync::{
    Arc, Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};

struct SourceOwner {
    partition: LocalAccountPartition,
    current: AtomicBool,
}
impl LocalAccountSource for SourceOwner {
    fn partition(&self) -> LocalAccountPartition {
        self.partition.clone()
    }
    fn check_current(&self) -> Result<(), LocalAccountSourceError> {
        if self.current.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(LocalAccountSourceError::Stale)
        }
    }
}

#[derive(Clone, Default)]
pub struct ScriptedLocalAccountFactory(Arc<Mutex<Option<Arc<SourceOwner>>>>);
impl ScriptedLocalAccountFactory {
    /// A new login or key owner, even for the same persistent partition.
    pub fn sign_in(&self, partition: LocalAccountPartition) {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = slot.take() {
            previous.current.store(false, Ordering::Release);
        }
        *slot = Some(Arc::new(SourceOwner {
            partition,
            current: AtomicBool::new(true),
        }));
    }
    pub fn close(&self) {
        if let Some(previous) = self.0.lock().unwrap_or_else(PoisonError::into_inner).take() {
            previous.current.store(false, Ordering::Release);
        }
    }
}
impl LocalAccountSourceFactory for ScriptedLocalAccountFactory {
    fn capture(&self) -> Result<Arc<dyn LocalAccountSource>, HubError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|source| source.clone() as Arc<dyn LocalAccountSource>)
            .ok_or(HubError::KeysLocked)
    }
}
