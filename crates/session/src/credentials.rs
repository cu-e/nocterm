//! Where saved answers to [`SecretRequest`]s are kept.
//!
//! Terminals depend on this port, not on the vault that implements it; the
//! composition root supplies the adapter.
use crate::{CredentialId, Secret, SecretRequest};
use futures::future::BoxFuture;

/// A store failure, worded for the person at the prompt.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("Credential not found.")]
    NotFound,
    #[error("Store locked or unavailable.")]
    Locked,
    #[error("The captured account is no longer current.")]
    Stale,
    #[error("This answer is never saved from a prompt.")]
    NotStorable,
    #[error("Credential operation failed: {0}")]
    Backend(#[source] nocterm_core::SharedCause),
    #[error("{0}")]
    Failed(String),
}

impl CredentialStoreError {
    /// Retains an adapter's concrete failure without flattening its cause chain.
    pub fn backend(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(nocterm_core::SharedCause::new(error))
    }
}

pub type CredentialFuture<T> = BoxFuture<'static, Result<T, CredentialStoreError>>;

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
    /// MFA codes, never are, and a vault key is only read: it is put in the
    /// vault through its own port, not from a prompt.
    pub fn is_storable(&self) -> bool {
        !matches!(
            self,
            SecretRequest::Interactive { .. } | SecretRequest::PrivateKey { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn a_credential_backend_keeps_its_original_typed_cause_when_cloned() {
        let error = CredentialStoreError::backend(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        ));
        let cloned = error.clone();
        let cause = cloned
            .source()
            .unwrap()
            .downcast_ref::<nocterm_core::SharedCause>()
            .unwrap();
        assert_eq!(
            cause
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
    }
}
