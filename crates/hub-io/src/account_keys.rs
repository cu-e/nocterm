//! The account key lock over the hub links: whether the keys of an account are open on this
//! device, opening them with the typed password, and closing them.
//!
//! It acts on the same account slot as the vault, so unlocking here opens the vault and
//! locking closes it. It does not own the vault's idle timer: only vault use restarts that.

use std::sync::Arc;

use futures::FutureExt as _;
use nocterm_core::error::SharedCause;
use nocterm_hubs_model::{AccountKeyError, AccountKeyFuture, AccountKeyLock, HubError, HubId};
use nocterm_session::Secret;
use nocterm_vault::{VaultAccess as _, VaultError, VaultStatus};

use crate::links::HubLinks;

/// Implements [`AccountKeyLock`] over the hub links.
pub struct AccountKeys {
    links: Arc<HubLinks>,
}

impl AccountKeys {
    pub fn new(links: Arc<HubLinks>) -> Self {
        Self { links }
    }
}

impl AccountKeyLock for AccountKeys {
    fn is_open(&self, hub: &HubId) -> bool {
        self.links
            .open_account(hub)
            .is_ok_and(|account| account.vault().status() == VaultStatus::Unlocked)
    }

    fn unlock(&self, hub: &HubId, password: Secret) -> AccountKeyFuture<()> {
        match self.links.open_account(hub) {
            Ok(account) => {
                let unlocking = account.vault().unlock(password);
                async move { unlocking.await.map_err(failure) }.boxed()
            }
            Err(error) => async move { Err(sign_in_failure(error)) }.boxed(),
        }
    }

    fn lock(&self, hub: &HubId) {
        if let Ok(account) = self.links.open_account(hub) {
            account.vault().lock();
        }
    }
}

fn failure(error: VaultError) -> AccountKeyError {
    match error {
        VaultError::Authentication => AccountKeyError::WrongPassword,
        VaultError::Offline => AccountKeyError::Offline,
        VaultError::Stale | VaultError::Cancelled => AccountKeyError::NotSignedIn,
        other => AccountKeyError::Failed {
            cause: SharedCause::new(other),
        },
    }
}

fn sign_in_failure(error: HubError) -> AccountKeyError {
    match error {
        HubError::NotSignedIn | HubError::Stale | HubError::NotFound => {
            AccountKeyError::NotSignedIn
        }
        other => AccountKeyError::Failed {
            cause: SharedCause::new(other),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_password_and_an_unreachable_hub_are_told_apart() {
        assert!(matches!(
            failure(VaultError::Authentication),
            AccountKeyError::WrongPassword
        ));
        assert!(matches!(
            failure(VaultError::Offline),
            AccountKeyError::Offline
        ));
        assert!(matches!(
            failure(VaultError::Stale),
            AccountKeyError::NotSignedIn
        ));
        let other = failure(VaultError::Format);
        assert!(matches!(other, AccountKeyError::Failed { .. }));
        assert!(std::error::Error::source(&other).is_some());
    }

    #[test]
    fn a_hub_without_a_sign_in_is_not_signed_in() {
        assert!(matches!(
            sign_in_failure(HubError::NotSignedIn),
            AccountKeyError::NotSignedIn
        ));
    }
}
