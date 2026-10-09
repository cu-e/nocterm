//! The vault as the [`CredentialStore`] terminals answer prompts from.
use futures::{FutureExt as _, TryFutureExt as _, future::BoxFuture};
use nocterm_session::{CredentialFuture, CredentialId, CredentialStore, Secret, SecretRequest};
use nocterm_vault::{CredentialBinding, VaultService};
use std::sync::Arc;

pub struct VaultCredentials(Arc<VaultService>);

impl VaultCredentials {
    pub fn new(service: Arc<VaultService>) -> Self {
        Self(service)
    }
}

/// The vault key a request's answer is stored under. One-time answers have
/// none, so the vault can never hold them.
fn binding(request: &SecretRequest) -> Option<CredentialBinding> {
    match request {
        SecretRequest::Password { target, .. } => Some(CredentialBinding::Password {
            target: target.clone(),
        }),
        SecretRequest::KeyPassphrase { path, .. } => {
            Some(CredentialBinding::KeyPassphrase { path: path.clone() })
        }
        SecretRequest::Interactive { .. } => None,
    }
}

fn not_storable<T: Send + 'static>() -> CredentialFuture<T> {
    futures::future::ready(Err("One-time answers are never saved.".to_owned())).boxed()
}

impl CredentialStore for VaultCredentials {
    fn is_available(&self) -> bool {
        self.0.is_unlocked()
    }

    fn wait_available(&self) -> BoxFuture<'static, ()> {
        self.0.wait_unlocked()
    }

    fn get(&self, id: CredentialId, request: &SecretRequest) -> CredentialFuture<Secret> {
        let Some(binding) = binding(request) else {
            return not_storable();
        };
        self.0
            .get(id, binding)
            .map_err(|error| error.to_string())
            .boxed()
    }

    fn save(
        &self,
        label: String,
        request: &SecretRequest,
        secret: Secret,
    ) -> CredentialFuture<CredentialId> {
        let Some(binding) = binding(request) else {
            return not_storable();
        };
        // A new ID avoids replacing a different binding referenced by an
        // edited profile.
        self.0
            .put(None, label, binding, secret)
            .map_err(|error| error.to_string())
            .boxed()
    }
}

#[cfg(test)]
mod tests;
