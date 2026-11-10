//! In-memory control of account keys, for tests of what unlocks and locks them.

use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt as _;
use nocterm_session::Secret;

use crate::account_keys::{AccountKeyError, AccountKeyFuture, AccountKeyLock};
use crate::address::HubId;
use crate::testing::FakeHub;

/// An in-memory [`AccountKeyLock`] over the key state of a [`FakeHub`], which also closes what
/// the hub's other fakes read ([`FakeHub::lock_keys`]). Opening needs `password`.
#[derive(Clone)]
pub struct FakeAccountKeys {
    hub: FakeHub,
    password: String,
    attempts: Arc<Mutex<Vec<bool>>>,
}

impl FakeAccountKeys {
    /// Keys of `hub` that open with `password`.
    pub fn alongside(hub: &FakeHub, password: &str) -> Self {
        Self {
            hub: hub.clone(),
            password: password.to_owned(),
            attempts: Arc::default(),
        }
    }

    /// Whether each unlock attempt so far was accepted, in order.
    pub fn attempts(&self) -> Vec<bool> {
        self.attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl AccountKeyLock for FakeAccountKeys {
    fn is_open(&self, hub: &HubId) -> bool {
        self.hub.keys_open(hub)
    }

    fn unlock(&self, hub: &HubId, password: Secret) -> AccountKeyFuture<()> {
        let (keys, hub) = (self.clone(), hub.clone());
        async move {
            let accepted = password.expose() == keys.password;
            keys.attempts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(accepted);
            if !keys.hub.is_signed_in(&hub) {
                return Err(AccountKeyError::NotSignedIn);
            }
            if !accepted {
                return Err(AccountKeyError::WrongPassword);
            }
            keys.hub.unlock_keys(&hub);
            Ok(())
        }
        .boxed()
    }

    fn lock(&self, hub: &HubId) {
        self.hub.lock_keys(hub);
    }
}
