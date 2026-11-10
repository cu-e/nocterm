//! An explicit login owns its original native selection and private device file.
use super::{Attempt, open};
use crate::{
    classify::{credential_file, remote},
    credentials::{CredentialFile, CredentialOrigin, Device},
    pool::{Pool, observation::LoginObservation},
    task::spawn,
};
use nocterm_hubs_model::{HubEndpointSpec, HubError, HubFuture, HubId};
use nocterm_keys::DeviceKey;
use std::sync::Arc;
use tokio::runtime::Handle;

pub(crate) struct LoginStart {
    spec: HubEndpointSpec,
    observed: Arc<LoginObservation>,
    file: CredentialFile,
    device: Option<Device>,
    origin: CredentialOrigin,
    created: bool,
    handed_off: bool,
}
impl LoginStart {
    pub(crate) fn capture(pool: Arc<Pool>, spec: HubEndpointSpec) -> Result<Self, HubError> {
        let observed = LoginObservation::capture(pool.clone(), HubId::from(spec.address.clone()))?;
        let file = pool.file(&spec.address)?;
        let (mut device, created) = match file.load().map_err(credential_file)? {
            Some(device) => (device, false),
            None => (
                Device {
                    key: DeviceKey::generate().map_err(remote)?,
                    session: None,
                    login: None,
                },
                true,
            ),
        };
        if let Err(error) = observed.check() {
            device.wipe_session();
            return Err(error);
        }
        let origin = device.origin();
        Ok(Self {
            spec,
            observed,
            file,
            device: Some(device),
            origin,
            created,
            handed_off: false,
        })
    }
    pub(crate) fn observation(&self) -> &Arc<LoginObservation> {
        &self.observed
    }
    pub(crate) fn start<T, F>(
        self,
        handle: &Handle,
        work: impl FnOnce(Self) -> F + Send + 'static,
    ) -> HubFuture<T>
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, HubError>> + Send + 'static,
    {
        let observed = self.observed.clone();
        let returned = observed.clone();
        let task = spawn(handle, async move { observed.during(work(self)).await? });
        Box::pin(async move {
            let result = task.await.unwrap_or(Err(HubError::Stale));
            returned.check()?;
            result
        })
    }
    pub(super) async fn open(mut self) -> Result<Attempt, HubError> {
        let device = self.device.as_ref().ok_or(HubError::Stale)?;
        self.observed.with_current(|| {
            self.file
                .prepare_login(device, self.created)
                .map_err(credential_file)
        })?;
        let mut device = self.device.take().ok_or(HubError::Stale)?;
        let client = self
            .observed
            .during(open(&self.spec, device.key, device.session.take()))
            .await??;
        self.observed.check()?;
        let attempt = Attempt {
            hub: self.observed.hub().clone(),
            file: self.file.clone(),
            client,
            trust: self.spec.trust,
            created: self.created,
            finished: false,
            origin: self.origin.clone(),
            observed: self.observed.clone(),
        };
        self.handed_off = true;
        Ok(attempt)
    }
}
impl Drop for LoginStart {
    fn drop(&mut self) {
        if let Some(device) = &mut self.device {
            device.wipe_session();
        }
        if self.created && !self.handed_off {
            let _ = self.file.erase_unpublished(self.origin.public_key());
        }
    }
}
