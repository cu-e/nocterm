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
pub struct ScriptedLocalAccountFactory(Arc<Mutex<Option<Arc<SourceOwner>>>>, Arc<AtomicBool>);
impl ScriptedLocalAccountFactory {
    /// A new login or key owner, even for the same persistent partition.
    pub fn sign_in(&self, partition: LocalAccountPartition) {
        self.1.store(false, Ordering::Release);
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(previous) = slot.take() {
            previous.current.store(false, Ordering::Release);
        }
        *slot = Some(Arc::new(SourceOwner {
            partition,
            current: AtomicBool::new(true),
        }));
    }
    /// The account is signed out: later captures fail as not signed in until a sign in.
    pub fn sign_out(&self) {
        self.close();
        self.1.store(true, Ordering::Release);
    }
    /// The keys lock: later captures fail as locked until a sign in.
    pub fn close(&self) {
        if let Some(previous) = self.0.lock().unwrap_or_else(PoisonError::into_inner).take() {
            previous.current.store(false, Ordering::Release);
        }
    }
}
impl LocalAccountSourceFactory for ScriptedLocalAccountFactory {
    fn capture(&self) -> Result<Arc<dyn LocalAccountSource>, HubError> {
        let signed_out = self.1.load(Ordering::Acquire);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|source| source.clone() as Arc<dyn LocalAccountSource>)
            .ok_or(if signed_out {
                HubError::NotSignedIn
            } else {
                HubError::KeysLocked
            })
    }
}
