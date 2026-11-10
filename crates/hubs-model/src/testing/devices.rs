//! The devices of fake hub accounts, for tests of what lists and revokes them.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt as _;
use futures::channel::oneshot;

use crate::address::HubId;
use crate::devices::{AccountDevices, DeviceSummary};
use crate::error::HubError;
use crate::ports::HubFuture;

/// An in-memory [`AccountDevices`]. Clones share one state, so a test keeps a handle while a
/// service owns another.
#[derive(Clone, Default)]
pub struct FakeAccountDevices {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    devices: Vec<(HubId, DeviceSummary)>,
    revoked: Vec<(HubId, String)>,
    failure: Option<HubError>,
    held: Option<oneshot::Receiver<()>>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeAccountDevices {
    /// The account on `hub` has these devices from now on.
    pub fn set(&self, hub: &HubId, devices: Vec<DeviceSummary>) {
        let mut state = lock(&self.state);
        state.devices.retain(|(known, _)| known != hub);
        state
            .devices
            .extend(devices.into_iter().map(|device| (hub.clone(), device)));
    }

    /// Every call fails with `error` until [`FakeAccountDevices::recover`].
    pub fn fail_with(&self, error: HubError) {
        lock(&self.state).failure = Some(error);
    }

    /// Calls work again.
    pub fn recover(&self) {
        lock(&self.state).failure = None;
    }

    /// The next listing waits until the returned sender is used or dropped.
    pub fn hold_next_list(&self) -> oneshot::Sender<()> {
        let (release, held) = oneshot::channel();
        lock(&self.state).held = Some(held);
        release
    }

    /// The devices revoked so far, in order.
    pub fn revoked(&self) -> Vec<(HubId, String)> {
        lock(&self.state).revoked.clone()
    }
}

impl AccountDevices for FakeAccountDevices {
    fn list(&self, hub: &HubId) -> HubFuture<Vec<DeviceSummary>> {
        let hub = hub.clone();
        let state = self.state.clone();
        let held = lock(&state).held.take();
        async move {
            if let Some(held) = held {
                let _ = held.await;
            }
            let state = lock(&state);
            if let Some(error) = &state.failure {
                return Err(error.clone());
            }
            Ok(state
                .devices
                .iter()
                .filter(|(known, _)| *known == hub)
                .map(|(_, device)| device.clone())
                .collect())
        }
        .boxed()
    }

    fn revoke(&self, hub: &HubId, device: &str) -> HubFuture<()> {
        let (hub, device) = (hub.clone(), device.to_owned());
        let state = self.state.clone();
        async move {
            let mut state = lock(&state);
            if let Some(error) = &state.failure {
                return Err(error.clone());
            }
            state
                .devices
                .retain(|(known, summary)| *known != hub || summary.id != device);
            state.revoked.push((hub, device));
            Ok(())
        }
        .boxed()
    }
}
