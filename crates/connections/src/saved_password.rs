//! The port through which the connection editor keeps the password a saved connection signs in
//! with, in the vault of its window: the account's vault on the hub in a hub window, this
//! computer's vault otherwise. A saved password may sign in to several servers; the editor
//! offers the vault's passwords and attaches the one chosen to the connection's address. The
//! composition root implements it over that vault.
use futures::future::BoxFuture;
use nocterm_session::{CredentialId, Secret, Target};

/// Which vault a window keeps its passwords and keys in, as the editor names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VaultPlace {
    /// The vault of this computer.
    #[default]
    ThisComputer,
    /// The account's vault on the window's hub, sealed end to end: every computer of the
    /// account signs in with what it keeps.
    Account,
}

/// Why a password was not kept, in words the editor shows.
#[derive(Debug, thiserror::Error)]
pub enum PasswordNotKept {
    #[error("Unlock the vault to keep the password.")]
    Locked,
    #[error("This window has no vault to keep the password in.")]
    Unavailable,
    #[error("The saved password is no longer in the vault.")]
    Missing,
    #[error("The vault could not keep the password.")]
    Vault(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// A password the vault keeps, as the editor offers it to sign in with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedPassword {
    pub id: CredentialId,
    pub label: String,
    /// How many servers it signs in to.
    pub servers: usize,
}

/// The vault the window keeps passwords in.
pub trait SavedPasswords: Send + Sync {
    /// Which vault it is.
    fn place(&self) -> VaultPlace;
    /// The passwords the vault keeps, in its order.
    fn list(&self) -> BoxFuture<'static, Result<Vec<SavedPassword>, PasswordNotKept>>;
    /// Keeps `password` for `target`: as the new password of the record `replacing`, which
    /// keeps its name and every server it signs in to and gains `target`, or in a new record
    /// named `label` that signs in to `target` only. Resolves to the record.
    fn keep(
        &self,
        replacing: Option<CredentialId>,
        label: String,
        target: Target,
        password: Secret,
    ) -> BoxFuture<'static, Result<CredentialId, PasswordNotKept>>;
    /// Lets the saved password `id` sign in to `target` as well.
    fn attach(
        &self,
        id: CredentialId,
        target: Target,
    ) -> BoxFuture<'static, Result<(), PasswordNotKept>>;
}

/// The port of a window composed without a vault: nothing is kept.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSavedPasswords;

impl SavedPasswords for NoSavedPasswords {
    fn place(&self) -> VaultPlace {
        VaultPlace::ThisComputer
    }

    fn list(&self) -> BoxFuture<'static, Result<Vec<SavedPassword>, PasswordNotKept>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }

    fn keep(
        &self,
        _: Option<CredentialId>,
        _: String,
        _: Target,
        _: Secret,
    ) -> BoxFuture<'static, Result<CredentialId, PasswordNotKept>> {
        Box::pin(std::future::ready(Err(PasswordNotKept::Unavailable)))
    }

    fn attach(
        &self,
        _: CredentialId,
        _: Target,
    ) -> BoxFuture<'static, Result<(), PasswordNotKept>> {
        Box::pin(std::future::ready(Err(PasswordNotKept::Unavailable)))
    }
}
