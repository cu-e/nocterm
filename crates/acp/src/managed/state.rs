//! Active native admission retains the original loan only until driver closure.
use super::{budget::SourceBudget, initialize::Restore};
use agent_client_protocol::managed::{ManagedError, Shared};
use nocterm_ai::{
    history::source::SourceLifetime,
    model::{
        json::deferred::{JsonDecodeError, NativeAllocationError},
        retention::RetentionError,
    },
};
use std::sync::Mutex;
struct Active {
    lifetime: Option<SourceLifetime>,
    restore: Restore,
    fork: bool,
    close: bool,
}
#[derive(Clone)]
pub(super) struct State(Shared<Mutex<Active>, SourceBudget>);
impl State {
    pub(super) fn new(
        lifetime: SourceLifetime,
        restore: Restore,
        fork: bool,
        close: bool,
        budget: &SourceBudget,
    ) -> Result<Self, ManagedError<JsonDecodeError>> {
        Ok(Self(Shared::new(
            Mutex::new(Active {
                lifetime: Some(lifetime),
                restore,
                fork,
                close,
            }),
            budget,
        )?))
    }
    pub(super) fn lifetime(&self) -> Result<SourceLifetime, NativeAllocationError> {
        self.0
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lifetime
            .as_ref()
            .cloned()
            .ok_or(NativeAllocationError::Retention(
                RetentionError::FrozenOwner,
            ))
    }
    pub(super) fn capabilities(&self) -> (Restore, bool, bool) {
        let active = self
            .0
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        (active.restore, active.fork, active.close)
    }
    pub(super) fn initialize(&self, restore: Restore, fork: bool, close: bool) {
        let mut active = self
            .0
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        active.restore = restore;
        active.fork = fork;
        active.close = close;
    }
    pub(super) fn close(&self) {
        let lifetime = self
            .0
            .get()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lifetime
            .take();
        drop(lifetime);
    }
}
