//! Whether the keys of an account are open on this device, and the ways to open or close them.
//!
//! Everything that reads the account's encrypted records (the agent access records, the
//! vault) needs open keys. A restart leaves them closed until the user types the password, and
//! the user may close them again at any time.

use futures::future::BoxFuture;
use nocterm_core::error::SharedCause;
use nocterm_session::Secret;

use crate::address::HubId;

/// Why the keys of an account could not be opened.
#[derive(Clone, Debug, thiserror::Error)]
pub enum AccountKeyError {
    /// The password does not open the keys.
    #[error("the password does not open the account keys")]
    WrongPassword,
    /// The hub cannot be reached and no copy of the keys' record is kept on this device.
    #[error("the hub cannot be reached")]
    Offline,
    /// This device is not signed in to the hub, or the sign-in ended meanwhile.
    #[error("this device is not signed in to the hub")]
    NotSignedIn,
    /// The account keys cannot be opened for another reason.
    #[error("the account keys cannot be opened")]
    Failed {
        #[source]
        cause: SharedCause,
    },
}

/// A result that completes later.
pub type AccountKeyFuture<T> = BoxFuture<'static, Result<T, AccountKeyError>>;

/// The keys of the account on a hub: their state, opening with the password, and closing.
pub trait AccountKeyLock: Send + Sync {
    /// Whether the keys of `hub`'s account are open now.
    fn is_open(&self, hub: &HubId) -> bool;

    /// Opens the keys with `password`: the login password, or the vault passphrase of an
    /// account that signs in through a provider. Already open keys stay open.
    fn unlock(&self, hub: &HubId, password: Secret) -> AccountKeyFuture<()>;

    /// Closes the keys: records that need them are unreadable until they are opened again.
    fn lock(&self, hub: &HubId);
}
