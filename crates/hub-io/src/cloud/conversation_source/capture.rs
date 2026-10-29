//! One original native account/keys plus the accepted SDK login generation.
use super::sdk_error;
use crate::{account::operations::BoundOperations, cloud::AccountChatMaterial};
use nocterm_cloud_model::projection::ProjectionError;
use nocterm_hub_client::{
    Keyset,
    cloud::{CloudClient, CloudError},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(crate) struct NativeSource {
    bound: BoundOperations,
    cloud: Arc<CloudClient>,
    material: Arc<AccountChatMaterial>,
}
impl NativeSource {
    pub(crate) fn new(
        bound: BoundOperations,
        cloud: Arc<CloudClient>,
        material: Arc<AccountChatMaterial>,
        pinned_hub: [u8; 32],
    ) -> Result<Arc<Self>, ProjectionError> {
        let source = Arc::new(Self {
            bound,
            cloud,
            material,
        });
        source.check()?;
        let request = source.material.request();
        let scope = request.stream.scope();
        let subject = source
            .bound
            .source()
            .native()
            .subject()
            .map_err(|_| ProjectionError::Stale)?;
        if subject.account() != source.cloud.subject().account().as_str()
            || subject.device() != source.cloud.subject().device().as_str()
            || scope.hub().instance().as_str() != source.cloud.namespace().hub_instance_id.as_str()
            || scope.account().id().as_str() != source.cloud.namespace().account_id.as_str()
            || scope.hub().authority().as_bytes() != &<[u8; 32]>::from(Sha256::digest(pinned_hub))
        {
            return Err(ProjectionError::WrongSource);
        }
        let key = source.with_keyset(|keys| Ok(keys.public_key().to_bytes()))?;
        if scope.account().key().as_bytes() != &<[u8; 32]>::from(Sha256::digest(key)) {
            return Err(ProjectionError::WrongSource);
        }
        Ok(source)
    }
    pub(super) fn cloud(&self) -> &CloudClient {
        &self.cloud
    }
    pub(super) fn material(&self) -> &Arc<AccountChatMaterial> {
        &self.material
    }
    pub(super) fn check(&self) -> Result<(), ProjectionError> {
        self.cloud.check_source().map_err(sdk_error)?;
        self.bound
            .source()
            .check()
            .map_err(|_| ProjectionError::Stale)?;
        self.material
            .check_current()
            .map_err(|_| ProjectionError::Stale)
    }
    pub(super) fn publish<T>(&self, action: impl FnOnce() -> T) -> Result<T, ProjectionError> {
        self.check()?;
        let result = self
            .bound
            .source()
            .publish(action)
            .map_err(|_| ProjectionError::Stale)?;
        self.check()?;
        Ok(result)
    }
    /// Private adapter composition only; neither account keys nor signing APIs
    /// cross the consumer port, and no strong keyset survives an await.
    pub(super) fn with_keyset<T>(
        &self,
        action: impl FnOnce(&Keyset) -> Result<T, CloudError>,
    ) -> Result<T, ProjectionError> {
        self.check()?;
        let result = self
            .bound
            .source()
            .native()
            .with_current(|keys| Ok(action(keys.keyset())))
            .map_err(|_| ProjectionError::Stale)?;
        self.check()?;
        result.map_err(sdk_error)
    }
    pub(super) async fn ended(self: Arc<Self>) {
        // No strong Slot/OpenKeys owner survives subscription construction.
        // Closing the actual account owner therefore also closes this receiver.
        let mut changes = match self.bound.source().native().slot() {
            Ok(slot) => slot.notice().subscribe(),
            Err(_) => return,
        };
        // Subscribe before checking, so replacement cannot be missed.
        loop {
            if self.check().is_err() {
                return;
            }
            if changes.changed().await.is_err() {
                return;
            }
        }
    }
}
