//! Captured per-open authority. Descriptive requests never acquire this capability.
use crate::{RelayDialer, SessionError};
use futures::{FutureExt as _, future::BoxFuture};
use std::{
    fmt,
    future::{Future, poll_fn},
    sync::Arc,
    task::Poll,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionSourceError {
    #[error("the original connection source was revoked")]
    Revoked,
    #[error("the original connection source is unavailable")]
    Unavailable,
    #[error("this transport does not support captured connection authority")]
    Unsupported,
}
/// Synchronous, nonblocking, non-I/O validation of the originally captured owner.
pub trait ConnectionSourceGuard: Send + Sync + 'static {
    fn check(&self) -> Result<(), ConnectionSourceError>;
}
#[derive(Clone, Default)]
pub struct ConnectionSource {
    guard: Option<Arc<dyn ConnectionSourceGuard>>,
}
impl ConnectionSource {
    pub fn new(guard: Arc<dyn ConnectionSourceGuard>) -> Self {
        Self { guard: Some(guard) }
    }
    pub fn is_restricted(&self) -> bool {
        self.guard.is_some()
    }
    /// Capability-owner identity, never equality of descriptive scope metadata.
    pub fn same_owner(&self, other: &Self) -> bool {
        match (&self.guard, &other.guard) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub fn check(&self) -> Result<(), ConnectionSourceError> {
        self.guard.as_ref().map_or(Ok(()), |guard| guard.check())
    }
    /// Recheck before every future poll and after completion. Cancellation does
    /// not undo transport work admitted by an earlier successful check.
    pub fn guard<'a, T, E, F>(&self, future: F) -> BoxFuture<'a, Result<T, E>>
    where
        T: Send + 'a,
        E: From<ConnectionSourceError> + Send + 'a,
        F: Future<Output = Result<T, E>> + Send + 'a,
    {
        let source = self.clone();
        let mut future = Box::pin(future);
        poll_fn(move |cx| {
            if let Err(error) = source.check() {
                return Poll::Ready(Err(error.into()));
            }
            match future.as_mut().poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(result) => Poll::Ready(source.check().map_err(Into::into).and(result)),
            }
        })
        .boxed()
    }
}
impl fmt::Debug for ConnectionSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionSource")
            .field("restricted", &self.is_restricted())
            .finish()
    }
}
/// Runtime-only collaborators, kept separate from serializable/Eq route facts.
#[derive(Clone, Default)]
pub struct ConnectionOpenOptions {
    source: ConnectionSource,
    relay: Option<Arc<dyn RelayDialer>>,
}
impl ConnectionOpenOptions {
    pub fn new(source: ConnectionSource) -> Self {
        Self {
            source,
            relay: None,
        }
    }
    pub fn with_relay(mut self, relay: Arc<dyn RelayDialer>) -> Self {
        self.relay = Some(relay);
        self
    }
    pub fn source(&self) -> &ConnectionSource {
        &self.source
    }
    pub fn relay(&self) -> Option<&Arc<dyn RelayDialer>> {
        self.relay.as_ref()
    }
    pub fn require_unrestricted(&self) -> Result<(), SessionError> {
        self.source.check()?;
        if self.source.is_restricted() || self.relay.is_some() {
            return Err(ConnectionSourceError::Unsupported.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
