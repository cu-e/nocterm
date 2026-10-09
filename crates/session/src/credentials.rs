//! Where saved answers to [`SecretRequest`]s are kept.
//!
//! Terminals depend on this port, not on the vault that implements it; the
//! composition root supplies the adapter.
use crate::{CredentialId, Secret, SecretRequest};
use futures::future::BoxFuture;

/// A store failure, worded for the person at the prompt.
pub type CredentialFuture<T> = BoxFuture<'static, Result<T, String>>;

pub trait CredentialStore: Send + Sync + 'static {
    /// Whether saved secrets can be read and written now.
    fn is_available(&self) -> bool;
    /// Completes once the store becomes available, without polling.
    fn wait_available(&self) -> BoxFuture<'static, ()>;
    /// The saved secret `id`, if it answers `request`.
    fn get(&self, id: CredentialId, request: &SecretRequest) -> CredentialFuture<Secret>;
    /// Saves `secret` as a new credential answering `request`.
    fn save(
        &self,
        label: String,
        request: &SecretRequest,
        secret: Secret,
    ) -> CredentialFuture<CredentialId>;
}

impl SecretRequest {
    /// Whether an answer may be saved and reused. One-time answers, such as
    /// MFA codes, never are.
    pub fn is_storable(&self) -> bool {
        !matches!(self, SecretRequest::Interactive { .. })
    }
}
