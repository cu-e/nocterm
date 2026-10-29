//! The vault as the [`CredentialStore`] terminals answer prompts from.
use futures::{FutureExt as _, TryFutureExt as _, future::BoxFuture};
use nocterm_session::{
    CredentialFuture, CredentialId, CredentialStore, CredentialStoreError, Secret, SecretRequest,
};
use nocterm_vault::{CredentialBinding, VaultAccess, VaultError};
use std::sync::Arc;

pub struct VaultCredentials(Arc<dyn VaultAccess>);

impl VaultCredentials {
    pub fn new(service: Arc<dyn VaultAccess>) -> Self {
        Self(service)
    }
}

fn failure(error: VaultError) -> CredentialStoreError {
    match error {
        VaultError::NotFound => CredentialStoreError::NotFound,
        VaultError::Locked => CredentialStoreError::Locked,
        VaultError::Stale => CredentialStoreError::Stale,
        other => CredentialStoreError::backend(other),
    }
}

fn not_storable<T: Send + 'static>() -> CredentialFuture<T> {
    futures::future::ready(Err(CredentialStoreError::NotStorable)).boxed()
}

impl CredentialStore for VaultCredentials {
    fn is_available(&self) -> bool {
        self.0.is_unlocked()
    }

    fn wait_available(&self) -> BoxFuture<'static, ()> {
        self.0.wait_unlocked()
    }

    fn get(&self, id: CredentialId, request: &SecretRequest) -> CredentialFuture<Secret> {
        let Some(binding) = CredentialBinding::for_request(request) else {
            return not_storable();
        };
        self.0.get(id, binding).map_err(failure).boxed()
    }

    fn save(
        &self,
        label: String,
        request: &SecretRequest,
        secret: Secret,
    ) -> CredentialFuture<CredentialId> {
        let Some(binding) =
            CredentialBinding::for_request(request).filter(|_| request.is_storable())
        else {
            return not_storable();
        };
        // A new ID avoids replacing a different binding referenced by an
        // edited profile.
        self.0
            .put(None, label, binding, secret)
            .map_err(failure)
            .boxed()
    }
}

#[cfg(test)]
mod tests;
