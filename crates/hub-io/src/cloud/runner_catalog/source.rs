//! Original native account source and finite signed-catalog highwater.
use super::{RunnerCatalogError, retained, sdk};
use crate::{
    account::operations::BoundOperations,
    cloud::{ChatMaterialPins, CloudAccountMaterial, RunnerRegistration},
};
use nocterm_cloud_model::{
    identity::{Fingerprint, Timestamp},
    journal::JournalCatalogBinding,
    tool::ToolTimer,
};
use nocterm_core::allocation::AllocationLease;
use nocterm_hub_client::{
    Keyset,
    cloud::{CloudClient, CloudError},
};
use nocterm_hub_model::cloud::head::{HeadProbeKeys, HeadProbeScope};
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::{
    runtime::Handle,
    sync::{OwnedSemaphorePermit, Semaphore},
};

#[derive(Clone, Copy)]
pub(super) struct PageStamp {
    pub revision: u64,
    pub root: Fingerprint,
    pub total: u32,
}
pub(super) struct CatalogSource {
    bound: BoundOperations,
    cloud: Arc<CloudClient>,
    binding: JournalCatalogBinding,
    pins: ChatMaterialPins,
    handle: Handle,
    timer: Arc<dyn ToolTimer>,
    state: Mutex<Observed>,
    operation: Arc<Semaphore>,
    resident: AllocationLease,
}
#[derive(Default)]
pub(super) struct Observed {
    last: Option<Timestamp>,
    clock_failed: bool,
    catalog: Option<PageStamp>,
}
impl CatalogSource {
    pub(super) fn capture(
        material: &CloudAccountMaterial,
        registration: &RunnerRegistration,
        resident: AllocationLease,
        handle: Handle,
        timer: Arc<dyn ToolTimer>,
    ) -> Result<Arc<Self>, RunnerCatalogError> {
        let (bound, cloud, authority) = material.conversation_parts();
        crate::cloud::runner_registry::validate_scope(
            &bound,
            &cloud,
            authority,
            registration.binding().scope(),
        )
        .map_err(RunnerCatalogError::Registry)?;
        let source = Arc::new(Self {
            bound,
            cloud,
            binding: registration.binding().clone(),
            pins: registration.pins(),
            handle,
            timer,
            state: Mutex::default(),
            operation: Arc::new(Semaphore::new(1)),
            resident,
        });
        source.check()?;
        Ok(source)
    }
    pub(super) fn check(&self) -> Result<(), RunnerCatalogError> {
        self.original()?;
        self.now().map(|_| ())
    }
    fn original(&self) -> Result<(), RunnerCatalogError> {
        self.bound
            .source()
            .check()
            .map_err(|error| self.account(error))?;
        self.cloud
            .check_source()
            .map_err(|error| retained(sdk(error), self.resident.clone()))
    }
    pub(super) fn now(&self) -> Result<i64, RunnerCatalogError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RunnerCatalogError::Unavailable)?;
        state.observe(self.timer.now())
    }
    pub(super) fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub(super) fn cloud(&self) -> &CloudClient {
        &self.cloud
    }
    pub(super) fn runtime(&self) -> &Handle {
        &self.handle
    }
    pub(super) fn turn(&self) -> Result<OwnedSemaphorePermit, RunnerCatalogError> {
        self.operation
            .clone()
            .try_acquire_owned()
            .map_err(|_| RunnerCatalogError::Capacity)
    }
    pub(super) fn with_keys<T>(
        &self,
        action: impl FnOnce(&Keyset) -> Result<T, CloudError>,
    ) -> Result<T, RunnerCatalogError> {
        self.check()?;
        let result = self
            .bound
            .source()
            .native()
            .with_current(|keys| Ok(action(keys.keyset())))
            .map_err(|error| self.account(error))?;
        self.check()?;
        result.map_err(sdk)
    }
    pub(super) fn scope(&self) -> Result<HeadProbeScope, RunnerCatalogError> {
        self.with_keys(|keys| {
            Ok(HeadProbeScope {
                namespace: self.cloud.namespace().clone(),
                runner: crate::cloud::metadata_crypto::runner(&self.binding),
                keys: HeadProbeKeys {
                    account: keys.public_key().to_bytes(),
                    runner_encryption: self.pins.encryption,
                    runner_signing: self.pins.signing,
                },
            })
        })
    }
    pub(super) async fn during<T>(
        &self,
        future: impl Future<Output = Option<T>>,
    ) -> Result<T, RunnerCatalogError> {
        self.bound
            .source()
            .during(future)
            .await
            .map_err(|error| self.account(error))?
            .ok_or(RunnerCatalogError::Unavailable)
    }
    pub(super) fn publish<T>(
        &self,
        stamp: PageStamp,
        action: impl FnOnce() -> T,
    ) -> Result<T, RunnerCatalogError> {
        self.check()?;
        let result = self
            .bound
            .source()
            .publish(|| {
                self.cloud.check_source().map_err(sdk)?;
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| RunnerCatalogError::Unavailable)?;
                state.observe(self.timer.now())?;
                state.accept(stamp)?;
                Ok(action())
            })
            .map_err(|error| self.account(error))??;
        self.check()?;
        Ok(result)
    }
    fn account(&self, error: nocterm_hubs_model::HubError) -> RunnerCatalogError {
        retained(
            RunnerCatalogError::Backend(crate::cloud::AccountMaterialError::Account(error)),
            self.resident.clone(),
        )
    }
}

impl Observed {
    pub(super) fn observe(&mut self, now: Timestamp) -> Result<i64, RunnerCatalogError> {
        if self.clock_failed || now.as_millis() == 0 || self.last.is_some_and(|old| now < old) {
            self.clock_failed = true;
            return Err(RunnerCatalogError::Clock);
        }
        let millis = i64::try_from(now.as_millis()).map_err(|_| {
            self.clock_failed = true;
            RunnerCatalogError::Clock
        })?;
        self.last = Some(now);
        Ok(millis)
    }
    pub(super) fn accept(&mut self, stamp: PageStamp) -> Result<(), RunnerCatalogError> {
        if self.clock_failed {
            return Err(RunnerCatalogError::Clock);
        }
        if stamp.revision == 0 {
            return Err(RunnerCatalogError::Format);
        }
        if self.catalog.is_some_and(|old| {
            stamp.revision < old.revision
                || (stamp.revision == old.revision
                    && (stamp.root != old.root || stamp.total != old.total))
        }) {
            return Err(RunnerCatalogError::Revision);
        }
        self.catalog = Some(stamp);
        Ok(())
    }
}
