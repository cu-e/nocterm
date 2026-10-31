//! Memory-only broker tokens, one per chat, held for the runner's life.
//!
//! The host hands a token over the control channel (ADR 0123). Nothing here
//! writes it anywhere: it lives in zeroizing strings and in the provider lease
//! built from it, and ends with REVOKE, a closed channel or the process.
use nocterm_agent_engine::{MAX_PROVIDER_LEASE, ProviderBinding, ProviderLease};
use nocterm_cloud_model::{engine::EngineError, identity::CloudChatId};
use nocterm_hub_model::agents::ChatAccessToken;
use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError},
};
use zeroize::Zeroizing;

/// Wakes the runs of a chat that parked waiting for provider access.
pub(crate) trait ProviderWake: Send + Sync {
    fn wake(&self, chat: CloudChatId);
}

struct Entry {
    token: Zeroizing<String>,
    lease: ProviderLease,
}
#[derive(Default)]
struct Held {
    closed: bool,
    chats: BTreeMap<CloudChatId, Entry>,
}

pub(crate) struct ChatTokens {
    binding: ProviderBinding,
    runtime: tokio::runtime::Handle,
    held: Mutex<Held>,
    wake: OnceLock<Arc<dyn ProviderWake>>,
}
impl ChatTokens {
    /// `runtime` is the runner's runtime: leases need it for their expiry task,
    /// and the control reader runs on a plain thread outside it.
    pub(crate) fn new(binding: ProviderBinding, runtime: tokio::runtime::Handle) -> Self {
        Self {
            binding,
            runtime,
            held: Mutex::new(Held::default()),
            wake: OnceLock::new(),
        }
    }
    /// Connects the coordinator once it exists. A second call is ignored.
    pub(crate) fn bind_wake(&self, wake: Arc<dyn ProviderWake>) {
        let _ = self.wake.set(wake);
    }
    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn lease_for(&self, token: &str) -> Result<ProviderLease, EngineError> {
        let _entered = self.runtime.enter();
        ProviderLease::new(
            self.binding.clone(),
            Zeroizing::new(token.to_owned()),
            MAX_PROVIDER_LEASE,
        )
    }
    /// Uses `token` for `chat` from now on, ending the access of any earlier
    /// token, then wakes the chat's parked runs. Refused once closed.
    pub(crate) fn issue(
        &self,
        chat: CloudChatId,
        token: &ChatAccessToken,
    ) -> Result<(), EngineError> {
        let lease = self.lease_for(token.expose())?;
        let previous = {
            let mut held = self.held();
            if held.closed {
                lease.revoke();
                return Err(EngineError::NeedsProviderAccess);
            }
            held.chats.insert(
                chat,
                Entry {
                    token: Zeroizing::new(token.expose().to_owned()),
                    lease,
                },
            )
        };
        if let Some(previous) = previous {
            previous.lease.revoke();
        }
        if let Some(wake) = self.wake.get() {
            wake.wake(chat);
        }
        Ok(())
    }
    /// Forgets the chat's token; its running model calls fail at their next send.
    pub(crate) fn revoke(&self, chat: CloudChatId) {
        let removed = self.held().chats.remove(&chat);
        if let Some(entry) = removed {
            entry.lease.revoke();
        }
    }
    /// Revokes every token and refuses new ones: the channel is gone.
    pub(crate) fn close(&self) {
        let chats = {
            let mut held = self.held();
            held.closed = true;
            std::mem::take(&mut held.chats)
        };
        for entry in chats.into_values() {
            entry.lease.revoke();
        }
    }
    /// The chat's current lease. A lease that lapsed by its time cap is rebuilt
    /// from the token still held, until REVOKE or close; the broker alone
    /// decides when the token stops being honoured.
    pub(crate) fn lease(&self, chat: CloudChatId) -> Result<ProviderLease, EngineError> {
        let mut held = self.held();
        if held.closed {
            return Err(EngineError::NeedsProviderAccess);
        }
        let entry = held
            .chats
            .get_mut(&chat)
            .ok_or(EngineError::NeedsProviderAccess)?;
        if !entry.lease.is_available() {
            entry.lease = self.lease_for(&entry.token)?;
        }
        Ok(entry.lease.clone())
    }
}
impl fmt::Debug for ChatTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let held = self.held();
        f.debug_struct("ChatTokens")
            .field("chats", &held.chats.len())
            .field("closed", &held.closed)
            .finish_non_exhaustive()
    }
}
