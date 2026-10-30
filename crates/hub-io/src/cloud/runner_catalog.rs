//! Fresh account/R catalog pages before any transcript content key is opened.
mod page;
mod source;
#[cfg(test)]
mod tests;
use super::{AccountMaterialError, CloudAccountMaterial, RunnerRegistration, RunnerRegistryError};
use futures::future::BoxFuture;
use nocterm_cloud_model::{
    identity::MAX_NAMESPACE_BYTES, journal::CurrentChatHead, tool::ToolTimer,
};
use nocterm_core::{SharedCause, allocation::AllocationLease};
use nocterm_hub_client::cloud::CloudError;
use nocterm_hub_model::cloud::head::MAX_HEAD_CATALOG_PAGE;
pub use page::RunnerCatalogPage;
use std::sync::Arc;
use tokio::runtime::Handle;

pub const RUNNER_CATALOG_RESIDENT_BYTES: usize = 16 * 1024;

/// One head row owns its stream scope and at most two original-run scopes.
/// Each scope contains two bounded native namespace strings. The Vec is created
/// at its final page capacity; SDK decoded data has a separate work owner.
pub fn runner_catalog_page_bytes() -> usize {
    std::mem::size_of::<RunnerCatalogPage>()
        + page::native_owner_bytes()
        + MAX_HEAD_CATALOG_PAGE * (std::mem::size_of::<CurrentChatHead>() + 6 * MAX_NAMESPACE_BYTES)
}
/// SDK logical request/response/decoded generations plus the temporary mapper,
/// canonical scope and original native binding. Connection/TLS baseline is separate.
pub fn runner_catalog_work_bytes() -> usize {
    nocterm_hub_client::cloud::head_query_work_bytes()
        + nocterm_hub_client::cloud::head_reply_work_bytes()
        + nocterm_hub_client::cloud::head_decoded_metadata_bytes()
        + 8 * MAX_NAMESPACE_BYTES
        + std::mem::size_of::<page::CatalogOperation>()
}
#[derive(Debug, thiserror::Error)]
pub enum RunnerCatalogError {
    #[error("the catalog belongs to another captured account or pinned runner")]
    Binding,
    #[error("the catalog operation exceeds its finite allocation bound")]
    Capacity,
    #[error("the original account catalog source is unavailable")]
    Unavailable,
    #[error("the catalog clock is permanently fenced")]
    Clock,
    #[error("the catalog revision changed or moved backwards; restart pagination")]
    Revision,
    #[error("the verified catalog page is inconsistent")]
    Format,
    #[error("the verified catalog has no continuation")]
    Complete,
    #[error("the account/R registration does not match the original account")]
    Registry(#[source] RunnerRegistryError),
    #[error("the authenticated catalog operation failed")]
    Backend(#[source] AccountMaterialError),
    #[error("this hub does not support authenticated account/R catalogs")]
    Unsupported(#[source] AccountMaterialError),
}

/// One original login/key owner and independently registered R pins. No ChatKey,
/// directory pin, provider credential or mutable account selector is retained.
pub struct AccountRunnerCatalog {
    source: Arc<source::CatalogSource>,
}
impl AccountRunnerCatalog {
    /// Supply a row from the authenticated personal registry or independently
    /// verified setup. This capture verifies original account/scope membership;
    /// it never turns directory-offered keys into trusted registration.
    pub fn capture(
        material: &CloudAccountMaterial,
        registration: &RunnerRegistration,
        resident: AllocationLease,
        handle: Handle,
        timer: Arc<dyn ToolTimer>,
    ) -> Result<Arc<Self>, RunnerCatalogError> {
        require(&resident, RUNNER_CATALOG_RESIDENT_BYTES)?;
        let source =
            source::CatalogSource::capture(material, registration, resident.clone(), handle, timer)
                .map_err(|error| retained(error, resident))?;
        Ok(Arc::new(Self { source }))
    }
    pub fn check_current(&self) -> Result<(), RunnerCatalogError> {
        self.source.check()
    }
    pub fn first(
        &self,
        work: AllocationLease,
        resident: AllocationLease,
    ) -> BoxFuture<'static, Result<RunnerCatalogPage, RunnerCatalogError>> {
        self.fetch(None, work, resident)
    }
    pub fn next(
        &self,
        previous: &RunnerCatalogPage,
        work: AllocationLease,
        resident: AllocationLease,
    ) -> BoxFuture<'static, Result<RunnerCatalogPage, RunnerCatalogError>> {
        let continuation = previous.continuation(&self.source);
        match continuation {
            Ok(cursor) => self.fetch(Some(cursor), work, resident),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }
    fn fetch(
        &self,
        after: Option<page::Continuation>,
        work: AllocationLease,
        resident: AllocationLease,
    ) -> BoxFuture<'static, Result<RunnerCatalogPage, RunnerCatalogError>> {
        let source = self.source.clone();
        let prepared = page::CatalogOperation::new(source.clone(), after, work.clone(), resident);
        Box::pin(async move {
            let operation = prepared?;
            source.check()?;
            let task = crate::task::spawn(source.runtime(), operation.execute());
            let output = source
                .during(task)
                .await
                .map_err(|error| retained(error, work))?;
            source.check()?;
            output.into_result()
        })
    }
}
fn require(lease: &AllocationLease, bytes: usize) -> Result<(), RunnerCatalogError> {
    if lease.bytes() < bytes || lease.payloads() == 0 {
        Err(RunnerCatalogError::Capacity)
    } else {
        Ok(())
    }
}
fn sdk(error: CloudError) -> RunnerCatalogError {
    let unsupported = matches!(
        &error,
        CloudError::Unsupported
            | CloudError::Capability(
                nocterm_hub_model::cloud::capabilities::CapabilityError::Unsupported
            )
    );
    let cause = AccountMaterialError::Backend(SharedCause::new(error));
    if unsupported {
        RunnerCatalogError::Unsupported(cause)
    } else {
        RunnerCatalogError::Backend(cause)
    }
}
fn retained(error: RunnerCatalogError, allocation: AllocationLease) -> RunnerCatalogError {
    match error {
        RunnerCatalogError::Backend(error)
        | RunnerCatalogError::Registry(RunnerRegistryError::Storage(error)) => {
            RunnerCatalogError::Backend(super::account_material::retain_error(error, allocation))
        }
        RunnerCatalogError::Unsupported(error) => RunnerCatalogError::Unsupported(
            super::account_material::retain_error(error, allocation),
        ),
        other => other,
    }
}
