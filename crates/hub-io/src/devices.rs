//! The devices of the account, listed and revoked through the original native membership.
//!
//! The membership is captured before the future is returned and rechecked after every await,
//! as in [`crate::connect`]'s sign-out: a result for an account that was signed out, replaced
//! or revoked meanwhile is [`HubError::Stale`] and is never shown for the new account.

use std::sync::Arc;

use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{DeviceSummary, HubError, HubFuture, HubId};
use tokio::runtime::Handle;

use crate::classify::classify;
use crate::pool::{Pool, observation::LoginObservation};
use crate::task::spawn;

/// The captured membership and its client.
struct Captured {
    observed: Arc<LoginObservation>,
    client: HubClient,
    hub: HubId,
}

impl Captured {
    fn new(pool: Arc<Pool>, hub: &HubId) -> Result<Self, HubError> {
        let observed = LoginObservation::capture(pool.clone(), hub.clone())?;
        let subject = observed.original_subject()?.ok_or(HubError::NotSignedIn)?;
        let client = pool.captured(hub, &subject)?;
        observed.check()?;
        Ok(Self {
            observed,
            client,
            hub: hub.clone(),
        })
    }
}

pub(crate) fn list(pool: Arc<Pool>, handle: &Handle, hub: &HubId) -> HubFuture<Vec<DeviceSummary>> {
    run(Captured::new(pool, hub), handle, |captured| async move {
        let listed = captured
            .observed
            .during(captured.client.directory().list_devices(v1::Empty {}))
            .await?
            .map_err(|status| classify(captured.hub.address(), ClientError::from(status)))?
            .into_inner();
        Ok(listed
            .devices
            .into_iter()
            .filter(|device| !device.revoked)
            .map(|device| DeviceSummary {
                id: device.device_id,
                name: device.name,
                platform: device.platform,
                created_at_ms: device.created_at_ms,
                last_seen_at_ms: device.last_seen_at_ms,
                is_current: device.current,
            })
            .collect())
    })
}

pub(crate) fn revoke(pool: Arc<Pool>, handle: &Handle, hub: &HubId, device: &str) -> HubFuture<()> {
    let request = v1::RevokeDeviceRequest {
        device_id: device.to_owned(),
    };
    run(Captured::new(pool, hub), handle, |captured| async move {
        captured
            .observed
            .during(captured.client.directory().revoke_device(request))
            .await?
            .map_err(|status| classify(captured.hub.address(), ClientError::from(status)))?;
        Ok(())
    })
}

/// Runs `work` on the adapter's threads once the membership was captured, and checks it again
/// before the result is returned.
fn run<T, F, Fut>(captured: Result<Captured, HubError>, handle: &Handle, work: F) -> HubFuture<T>
where
    T: Send + 'static,
    F: FnOnce(Arc<Captured>) -> Fut,
    Fut: std::future::Future<Output = Result<T, HubError>> + Send + 'static,
{
    let captured = match captured {
        Ok(captured) => Arc::new(captured),
        Err(error) => return Box::pin(async move { Err(error) }),
    };
    let observed = captured.observed.clone();
    let task = spawn(handle, work(captured));
    Box::pin(async move {
        let result = task.await.unwrap_or(Err(HubError::Stale));
        observed.check()?;
        result
    })
}
