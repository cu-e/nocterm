//! Original native runner authority across actual TLS credential rotation.
use super::{Hub, hub};
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    identity::{RunnerId, RunnerKeyEpoch},
    journal::PublicationBudget,
};
use nocterm_hub_client::{DeviceInfo, HubClient, HubEndpoint, Keyset, cloud::CloudClient};
use nocterm_hub_io::cloud::{
    CloudRunnerIdentity, RunnerHubConnection, RunnerHubError, RunnerHubPeer, RunnerHubTrust,
};
use nocterm_keys::DeviceKey;
use std::{error::Error, future::Future, pin::Pin, sync::Arc, task::Poll, time::Duration};
type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct Fixture {
    _directory: tempfile::TempDir,
    device: HubClient,
    cloud: CloudClient,
    connection: Arc<RunnerHubConnection>,
    runner: [u8; 16],
}
impl Fixture {
    async fn start(hub: Hub) -> TestResult<Self> {
        let authority = std::env::var("NOCTERM_CLOUD_AUTHORITY")
            .map_err(|_| "NOCTERM_CLOUD_AUTHORITY is required for runner authority tests")?;
        let authority = nocterm_hub_client::parse_fingerprint(&authority)?;
        let fingerprint = hub.fingerprint.to_string();
        let endpoint = HubEndpoint::new(&hub.address.to_string(), Some(&fingerprint))?;
        let device = HubClient::connect(&endpoint, DeviceKey::generate()?, None).await?;
        let signed = device
            .password_login(
                &hub.username,
                &hub.password,
                &DeviceInfo::new("test", "native runner"),
            )
            .await?;
        let keyset = keyset(&device, &signed).await?;
        let cloud = device.cloud_offer().await?.accept(&authority)?;
        let runner = *uuid::Uuid::new_v4().as_bytes();
        let trust = RunnerHubTrust::new(hub.spec(), device.info().hub_id.clone(), authority);
        let binding = trust.binding(
            keyset.account_id(),
            keyset.public_key().to_bytes(),
            RunnerId::from_bytes(runner),
            RunnerKeyEpoch::new(1)?,
        )?;
        let directory = tempfile::tempdir()?;
        let identity = Arc::new(CloudRunnerIdentity::create_new(
            &directory.path().join("identity"),
            binding.clone(),
        )?);
        let ticket = cloud
            .create_runner_grant(runner, *identity.public_identity()?.proof_key())
            .await?;
        let connection = RunnerHubConnection::new(
            trust,
            binding,
            identity,
            &directory.path().join("session"),
            Arc::new(PublicationBudget::maintenance(IntakePoolLimits::new(
                128 * 1024,
                2,
                2,
            )?)),
        )?;
        connection
            .redeem(zeroize::Zeroizing::new(ticket.token().to_owned()))
            .await?;
        Ok(Self {
            _directory: directory,
            device,
            cloud,
            connection,
            runner,
        })
    }
    async fn revoke(&self) -> TestResult {
        assert!(self.cloud.revoke_runner(self.runner).await?);
        Ok(())
    }
}
async fn keyset(device: &HubClient, signed: &nocterm_hub_client::SignedIn) -> TestResult<Keyset> {
    if let Some(keyset) = device
        .open_keyset(&signed.session.account_id, signed.secrets.unlock())
        .await?
    {
        return Ok(keyset);
    }
    match device
        .create_keyset(
            &signed.session.account_id,
            signed.secrets.unlock(),
            signed.secrets.kdf(),
        )
        .await
    {
        Ok(created) => Ok(created.keyset),
        Err(nocterm_hub_client::KeysetError::AlreadyExists) => device
            .open_keyset(&signed.session.account_id, signed.secrets.unlock())
            .await?
            .ok_or_else(|| "the concurrently created account keyset is absent".into()),
        Err(error) => Err(error.into()),
    }
}

async fn observe_rotation<F>(source: &RunnerHubConnection, mut operation: Pin<&mut F>) -> TestResult
where
    F: Future<Output = Result<RunnerHubPeer, RunnerHubError>>,
{
    tokio::time::timeout(
        Duration::from_secs(10),
        futures::future::poll_fn(|cx| {
            if let Poll::Ready(result) = operation.as_mut().poll(cx) {
                return Poll::Ready(match result {
                    Ok(_) => {
                        Err("rotation completed without observing its closed transport".into())
                    }
                    Err(error) => Err(Box::new(error) as Box<dyn Error>),
                });
            }
            match source.check_source() {
                Err(RunnerHubError::DurabilityPending) => Poll::Ready(Ok(())),
                Err(error) => Poll::Ready(Err(Box::new(error) as Box<dyn Error>)),
                Ok(()) => Poll::Pending,
            }
        }),
    )
    .await?
}

#[tokio::test]
async fn runner_runtime_source_survives_actual_rotation_and_refuses_remote_revocation() -> TestResult
{
    let Some(hub) = hub() else { return Ok(()) };
    let fixture = Fixture::start(hub).await?;
    let source = &fixture.connection;
    source.check_runtime_source()?;
    let refresh = source.refresh();
    tokio::pin!(refresh);
    observe_rotation(source, refresh.as_mut()).await?;
    // The actual credential operation is pending on this current-thread runtime.
    // Strict RPC admission is closed, while the original run remains current.
    assert!(matches!(
        source.check_source(),
        Err(RunnerHubError::DurabilityPending)
    ));
    source.check_runtime_source()?;
    assert!(!source.transport_ready()?);
    let ready = source.wait_transport_ready();
    tokio::pin!(ready);
    assert!(futures::poll!(ready.as_mut()).is_pending());
    let (renewed, ready) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(refresh.as_mut(), ready.as_mut())
    })
    .await?;
    renewed?;
    ready?;
    source.check_runtime_source()?;
    source.check_source()?;
    assert!(source.transport_ready()?);
    fixture.revoke().await?;
    assert!(
        tokio::time::timeout(Duration::from_secs(10), source.refresh())
            .await?
            .is_err()
    );
    assert!(source.check_runtime_source().is_err());
    assert!(source.wait_transport_ready().await.is_err());
    source.retire_local();
    fixture.device.sign_out();
    Ok(())
}

#[tokio::test]
async fn cancelled_actual_runner_refresh_retires_the_original_runtime_and_wakes_waiters()
-> TestResult {
    let Some(hub) = hub() else { return Ok(()) };
    let fixture = Fixture::start(hub).await?;
    let source = &fixture.connection;
    let mut refresh = Box::pin(source.refresh());
    observe_rotation(source, refresh.as_mut()).await?;
    source.check_runtime_source()?;
    let waiting = source.wait_transport_ready();
    tokio::pin!(waiting);
    assert!(futures::poll!(waiting.as_mut()).is_pending());
    drop(refresh);
    assert!(source.check_runtime_source().is_err());
    assert!(
        tokio::time::timeout(Duration::from_secs(1), waiting)
            .await?
            .is_err()
    );
    assert!(source.refresh().await.is_err());
    fixture.revoke().await?;
    source.retire_local();
    fixture.device.sign_out();
    Ok(())
}
