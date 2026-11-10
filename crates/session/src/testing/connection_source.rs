//! Revocable source shared by native transport behavior tests.
use crate::{ConnectionSource, ConnectionSourceError, ConnectionSourceGuard};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Default)]
pub struct ScriptedConnectionSource {
    revoked: Arc<AtomicBool>,
}
impl ScriptedConnectionSource {
    pub fn source(&self) -> ConnectionSource {
        ConnectionSource::new(Arc::new(self.clone()))
    }
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
}
impl ConnectionSourceGuard for ScriptedConnectionSource {
    fn check(&self) -> Result<(), ConnectionSourceError> {
        if self.revoked.load(Ordering::Acquire) {
            Err(ConnectionSourceError::Revoked)
        } else {
            Ok(())
        }
    }
}
