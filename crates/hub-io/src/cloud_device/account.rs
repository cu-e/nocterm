//! One signed-in, unlocked account as the device core works with it: its keys, its hub
//! session (hub trust, verified capabilities, trusted runners) and the effects of its device.
//!
//! The core holds a copy of the unlocked account keys for as long as its flows run
//! (`AccountKeys`, zeroed on drop). Every owner of a flow watches [`DeviceAccount::ended`] and
//! drops its flows when the account is locked, signed out or replaced, so the copy never
//! outlives the unlocked keys.
use super::driver::{self, Control};
use super::effects::{EffectRunner, HubEffects};
use crate::account::operations::BoundOperations;
use crate::devices::Captured;
use crate::hub_account::HubAccount;
use nocterm_cloud_device::context::{AccountKeys, CloudSession};
use nocterm_cloud_device::discover::{Discover, Discovery};
use nocterm_cloud_device::records::TrustedRunner;
use nocterm_cloud_device::{CloudDeviceError, DeviceIdentity};
use nocterm_hubs_model::HubError;
use std::sync::{Arc, Mutex, PoisonError};

/// Why the account could not be opened for Cloud chats.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeviceAccountError {
    #[error(transparent)]
    Hub(#[from] HubError),
    /// Cloud chats were never set up for this account from a native device.
    #[error("Cloud chats are not set up for this account")]
    NotSetUp,
    #[error("this hub does not offer Cloud chats")]
    Unsupported,
    #[error("the account's Cloud records were refused")]
    Device(#[source] CloudDeviceError),
}

/// The account, its device and its Cloud session.
pub(crate) struct DeviceAccount {
    captured: Arc<Captured>,
    bound: BoundOperations,
    effects: Arc<dyn EffectRunner>,
    session: Mutex<Arc<CloudSession>>,
}
impl DeviceAccount {
    /// Captures the signed-in, unlocked account and discovers its Cloud session on the hub.
    pub(crate) async fn open(
        captured: Arc<Captured>,
        account: &HubAccount,
    ) -> Result<Arc<Self>, DeviceAccountError> {
        let source = account.local_source()?;
        let bound = account.bind(source)?;
        let keys = bound.source().native().with_current(|keys| {
            let keyset = keys.keyset();
            Ok(AccountKeys::new(
                keyset.account_id(),
                keyset.account_key().clone(),
                keyset.key_pair().clone(),
            ))
        })?;
        // The client signs every call with the device key it holds; nothing here reads it.
        captured
            .client
            .cloud_device()
            .map_err(|_| HubError::NotSignedIn)?;
        let effects: Arc<dyn EffectRunner> = Arc::new(HubEffects::new(captured.client.clone()));
        let raw_hub = captured.client.info().hub_id.clone();
        let mut discover = Discover::new(Arc::new(keys), &raw_hub, Some(captured.hub.to_string()));
        let mut found = None;
        driver::run(&mut discover, effects.clone(), |_, discovery| {
            found = Some(discovery);
            Control::Stop
        })
        .await
        .map_err(DeviceAccountError::Device)?;
        captured.observed.check()?;
        bound.source().check()?;
        let session = match found {
            Some(Discovery::Ready(session)) => session,
            Some(Discovery::Unsupported) => return Err(DeviceAccountError::Unsupported),
            Some(Discovery::NotSetUp | Discovery::ChooseOrigin(_)) | None => {
                return Err(DeviceAccountError::NotSetUp);
            }
        };
        Ok(Arc::new(Self {
            captured,
            bound,
            effects,
            session: Mutex::new(session),
        }))
    }
    /// The session; a chat created on a runner the registry did not hold yet adds it.
    pub(crate) fn session(&self) -> Arc<CloudSession> {
        self.session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    /// Adopts the trusted runners a flow read again from the registry.
    pub(crate) fn adopt_runners(&self, runners: Vec<TrustedRunner>) {
        let mut session = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        *session = Arc::new(session.with_runners(runners));
    }
    pub(crate) fn effects(&self) -> &Arc<dyn EffectRunner> {
        &self.effects
    }
    /// The signed-in device with its current access token.
    pub(crate) fn device(&self) -> Result<DeviceIdentity, HubError> {
        self.check()?;
        self.captured
            .client
            .cloud_device()
            .map_err(|_| HubError::NotSignedIn)
    }
    /// Fails with `Stale` once the account was locked, signed out or replaced.
    pub(crate) fn check(&self) -> Result<(), HubError> {
        self.captured.observed.check()?;
        self.bound.source().check()
    }
    /// Resolves when the account is locked, signed out or replaced.
    pub(crate) async fn ended(self: Arc<Self>) {
        let Ok(slot) = self.bound.source().native().slot() else {
            return;
        };
        let mut changes = slot.notice().subscribe();
        drop(slot);
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
