//! Terminal credentials bound to one native account capture, including worker I/O.
use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use nocterm_hubs_model::{HubError, source_identity::LocalAccountSource};
use nocterm_session::{
    CredentialFuture, CredentialId, CredentialStore, CredentialStoreError, Secret, SecretRequest,
};
use nocterm_vault::{CredentialBinding, VaultError};
use tokio::runtime::Handle;

use super::ledger::Ledger;
use crate::{
    account::{
        operations::{OpenedSource, OperationStores},
        source::LocalSourceProvider,
    },
    task::spawn,
};

/// Retained by HubAccount so capture can be retried after an ordinary unlock.
pub(crate) struct CredentialCapture {
    handle: Handle,
    operations: Arc<dyn OperationStores>,
    idle: Weak<super::Idle>,
}

impl CredentialCapture {
    pub(crate) fn new(
        handle: Handle,
        operations: Arc<dyn OperationStores>,
        idle: &Arc<super::Idle>,
    ) -> Self {
        Self {
            handle,
            operations,
            idle: Arc::downgrade(idle),
        }
    }

    pub(crate) fn capture(
        &self,
        provider: &LocalSourceProvider,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<dyn CredentialStore>, HubError> {
        source.check_current().map_err(|_| HubError::NotSignedIn)?;
        let context = OpenedSource::new(provider.capture_native()?, source)?;
        context.check_current().map_err(|_| HubError::NotSignedIn)?;
        Ok(Arc::new(CapturedCredentials {
            handle: self.handle.clone(),
            operations: Arc::downgrade(&self.operations),
            context,
            idle: self.idle.clone(),
        }))
    }
}

struct CapturedCredentials {
    handle: Handle,
    operations: Weak<dyn OperationStores>,
    context: Arc<OpenedSource>,
    idle: Weak<super::Idle>,
}
impl CapturedCredentials {
    fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(Ledger) -> BoxFuture<'static, Result<T, VaultError>> + Send + 'static,
    ) -> CredentialFuture<T> {
        let context = Arc::clone(&self.context);
        let returned = Arc::clone(&context);
        let operations = self.operations.clone();
        let idle = self.idle.clone();
        let task = spawn(&self.handle, async move {
            context
                .check_current()
                .map_err(|_| CredentialStoreError::Stale)?;
            let operations = operations.upgrade().ok_or(CredentialStoreError::Stale)?;
            // Only an admitted worker retains keys and the actual original
            // client; the facade itself owns weak references to both.
            let keys = context
                .native()
                .keys()
                .map_err(|_| CredentialStoreError::Stale)?;
            let slot = context
                .native()
                .slot()
                .map_err(|_| CredentialStoreError::Stale)?;
            let ops = operations
                .opened(context.native(), context.clone())
                .map_err(hub_failure)?;
            let ledger = Ledger::captured(slot, ops, context.native().clone());
            let result = work(ledger).await;
            context
                .check_current()
                .map_err(|_| CredentialStoreError::Stale)?;
            if let Some(idle) = idle.upgrade() {
                idle.touch(context.native());
            }
            drop(keys);
            result.map_err(failure)
        });
        Box::pin(async move {
            let result = task.await.ok_or(CredentialStoreError::Stale)?;
            // The worker may have finished before this future was first polled.
            returned
                .check_current()
                .map_err(|_| CredentialStoreError::Stale)?;
            result
        })
    }
}

fn hub_failure(error: HubError) -> CredentialStoreError {
    match error {
        HubError::NotSignedIn | HubError::KeysLocked | HubError::Stale => {
            CredentialStoreError::Stale
        }
        other => CredentialStoreError::backend(other),
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
    Box::pin(async { Err(CredentialStoreError::NotStorable) })
}

impl CredentialStore for CapturedCredentials {
    fn is_available(&self) -> bool {
        self.operations.upgrade().is_some() && self.context.check_current().is_ok()
    }

    fn wait_available(&self) -> BoxFuture<'static, ()> {
        // A capture never revives. A new attempt obtains a new capture after
        // unlock/login, instead of waiting on an unrelated future account.
        if self.is_available() {
            Box::pin(async {})
        } else {
            Box::pin(futures::future::pending())
        }
    }

    fn get(&self, id: CredentialId, request: &SecretRequest) -> CredentialFuture<Secret> {
        let Some(binding) = CredentialBinding::for_request(request) else {
            return not_storable();
        };
        self.run(move |ledger| Box::pin(async move { ledger.get(id, binding).await }))
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
        self.run(move |ledger| {
            Box::pin(async move { ledger.put(None, label, binding, secret).await })
        })
    }
}

#[cfg(test)]
mod tests;
