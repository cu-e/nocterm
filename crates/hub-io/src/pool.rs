//! The clients of the hubs this device is signed in to, and where it keeps their credentials.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use nocterm_hub_client::{ClientError, DeviceInfo, HubClient};
use nocterm_hubs_model::{HubAddress, HubError, HubId, TrustPolicy};

use crate::classify::{classify, credential_file};
use crate::credentials::{CredentialDir, CredentialFile};

/// The application name a hub lists beside the device.
const APP_NAME: &str = "Nocterm";

/// The client of a hub and the trust it was connected with.
struct Pooled {
    trust: TrustPolicy,
    client: HubClient,
}

/// One client per hub: clients of a hub share the turn in which its tokens are renewed, and
/// two of them would renew the same refresh token against each other.
pub(crate) struct Pool {
    credentials: CredentialDir,
    device: DeviceInfo,
    clients: Mutex<HashMap<HubId, Pooled>>,
}

impl Pool {
    pub(crate) fn new(credentials_dir: PathBuf, device_name: &str) -> Self {
        Self {
            credentials: CredentialDir::new(credentials_dir),
            device: DeviceInfo::new(device_name, APP_NAME),
            clients: Mutex::default(),
        }
    }

    pub(crate) fn device(&self) -> &DeviceInfo {
        &self.device
    }

    pub(crate) fn file(&self, address: &HubAddress) -> Result<CredentialFile, HubError> {
        self.credentials.file(address).map_err(credential_file)
    }

    fn clients(&self) -> MutexGuard<'_, HashMap<HubId, Pooled>> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn insert(&self, hub: HubId, trust: TrustPolicy, client: HubClient) {
        self.clients().insert(hub, Pooled { trust, client });
    }

    /// The client of the hub if it was connected with this trust.
    pub(crate) fn connected(&self, hub: &HubId, trust: TrustPolicy) -> Option<HubClient> {
        let clients = self.clients();
        let pooled = clients.get(hub)?;
        (pooled.trust == trust).then(|| pooled.client.clone())
    }

    pub(crate) fn evict(&self, hub: &HubId) -> Option<HubClient> {
        self.clients().remove(hub).map(|pooled| pooled.client)
    }

    /// The client of a hub connected before, with tokens that are good for a while.
    ///
    /// A hub that was not connected yet is reported as not signed in: the caller has to
    /// connect, with the trust it decided, before it can ask for anything.
    pub(crate) async fn fresh(&self, hub: &HubId) -> Result<HubClient, HubError> {
        let client = self
            .clients()
            .get(hub)
            .map(|pooled| pooled.client.clone())
            .ok_or(HubError::NotSignedIn)?;
        match client.ensure_fresh().await {
            Ok(_) => Ok(client),
            Err(error) => Err(self.failed(hub, error)),
        }
    }

    /// Classifies a failure on a hub; a revoked device loses its credentials on the spot.
    pub(crate) fn failed(&self, hub: &HubId, error: ClientError) -> HubError {
        if matches!(error, ClientError::Revoked) {
            self.evict(hub);
            if let Ok(file) = self.file(hub.address())
                && let Err(error) = file.erase()
            {
                tracing::warn!(%error, hub = %hub, "cannot erase the credentials of a revoked device");
            }
        }
        classify(hub.address(), error)
    }
}
