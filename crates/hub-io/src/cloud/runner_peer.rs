//! Independent runner credentials; desktop membership never owns this lifetime.
mod rotation;
mod runtime;
mod state;
mod transport;
use super::{CloudRunnerIdentity, CloudRunnerIdentityError};
use nocterm_cloud_model::{
    identity::{
        AccountIdentity, AccountScope, Fingerprint, HubIdentity, NamespaceId, RunnerId,
        RunnerKeyEpoch,
    },
    journal::{JournalCatalogBinding, PublicationWorkBudget},
};
use nocterm_core::SharedCause;
use nocterm_hub_client::{
    HubEndpoint,
    cloud::{CloudError, CloudRunner},
};
use nocterm_hub_model::cloud::{CloudTextId, hub_namespace_id, runner_auth::RunnerProofSubject};
use nocterm_hubs_model::HubEndpointSpec;
use nocterm_keys::DeviceKey;
use rotation::RefreshGuard;
use sha2::{Digest, Sha256};
use state::SessionOwner;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
pub(in crate::cloud) use transport::CapturedTransport;
use zeroize::Zeroizing;

use super::runner_session_file::{RunnerSessionFile, SessionRecord};

#[derive(Debug, thiserror::Error)]
pub enum RunnerHubError {
    #[error("the runner hub or identity differs from the original source")]
    Binding,
    #[error("the independent runner source is retired or unavailable")]
    Unavailable,
    #[error("this hub does not support the required Cloud runner capability")]
    Unsupported(#[source] SharedCause),
    #[error("the original runner credentials are no longer available")]
    SourceChanged(#[source] SharedCause),
    #[error("a runner session is already present; explicit restore is required")]
    Existing,
    #[error("runner credential rotation awaits a confirmed private durable session")]
    DurabilityPending,
    #[error("the original runner refresh has an unknown outcome")]
    RefreshUnknown,
    #[error("the original runner hard credential deadline expired")]
    HardExpired,
    #[error("the original runner clock moved backwards or is unavailable")]
    Clock,
    #[error("the original native transport generation changed; retry its immutable request")]
    TransportChanged,
    #[error("the runner session file is malformed or belongs to another source")]
    Format,
    #[error("the private runner session could not be accessed")]
    Storage(#[source] SharedCause),
    #[error("the runner session was published but durability could not be confirmed")]
    StoredButUnavailable(#[source] SharedCause),
    #[error("the runner hub refused or could not complete the operation")]
    Remote(#[source] SharedCause),
    #[error(transparent)]
    Identity(#[from] CloudRunnerIdentityError),
}

/// Native trust configuration captured during runner setup, never from a relay reply.
pub struct RunnerHubTrust {
    endpoint: HubEndpointSpec,
    raw_hub_id: String,
    authority: [u8; 32],
}
impl RunnerHubTrust {
    pub fn new(endpoint: HubEndpointSpec, raw_hub_id: String, authority: [u8; 32]) -> Self {
        Self {
            endpoint,
            raw_hub_id,
            authority,
        }
    }

    /// Derive native identity from independently pinned setup facts through the
    /// canonical public namespace codec. A relay reply never selects these pins.
    pub fn binding(
        &self,
        account_id: &str,
        account_public: [u8; 32],
        runner: RunnerId,
        epoch: RunnerKeyEpoch,
    ) -> Result<JournalCatalogBinding, RunnerHubError> {
        let raw = CloudTextId::new(&self.raw_hub_id).map_err(remote)?;
        let account = CloudTextId::new(account_id).map_err(remote)?;
        nocterm_keys::PublicKey::from_bytes(&account_public).map_err(remote)?;
        let authority_digest: [u8; 32] = Sha256::digest(self.authority).into();
        let instance = hub_namespace_id(raw.as_str(), authority_digest).map_err(remote)?;
        Ok(JournalCatalogBinding::new(
            AccountScope::new(
                HubIdentity::new(
                    NamespaceId::new(instance.as_str()).map_err(remote)?,
                    Fingerprint::from_bytes(authority_digest),
                ),
                AccountIdentity::new(
                    NamespaceId::new(account.as_str()).map_err(remote)?,
                    Fingerprint::from_bytes(Sha256::digest(account_public).into()),
                ),
            ),
            runner,
            epoch,
        ))
    }
}

/// Public scheduling facts from the current actual R session. No credential
/// string or mutable authentication authority crosses this boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunnerCredentialDeadlines {
    access: i64,
    refresh: i64,
}
impl RunnerCredentialDeadlines {
    pub fn access_expires_at_ms(self) -> i64 {
        self.access
    }
    pub fn refresh_expires_at_ms(self) -> i64 {
        self.refresh
    }
}

/// A process-exclusive runner session owner, independent from device sign-in/out.
/// The directory must have trusted private ancestors, as for runner identity.
pub struct RunnerHubConnection {
    endpoint: HubEndpoint,
    binding: JournalCatalogBinding,
    identity: Arc<CloudRunnerIdentity>,
    subject: RunnerProofSubject,
    authority: [u8; 32],
    work: Arc<dyn PublicationWorkBudget>,
    session: Mutex<SessionOwner>,
    file: Mutex<RunnerSessionFile>,
    notice: tokio::sync::watch::Sender<()>,
    operation: tokio::sync::Mutex<()>,
    retired: AtomicBool,
}
impl RunnerHubConnection {
    pub fn new(
        trust: RunnerHubTrust,
        binding: JournalCatalogBinding,
        identity: Arc<CloudRunnerIdentity>,
        private_directory: &Path,
        work: Arc<dyn PublicationWorkBudget>,
    ) -> Result<Arc<Self>, RunnerHubError> {
        if identity.binding() != &binding
            || <[u8; 32]>::from(Sha256::digest(trust.authority))
                != *binding.scope().hub().authority().as_bytes()
        {
            return Err(RunnerHubError::Binding);
        }
        let public = identity.public_identity()?;
        let raw_hub_id = CloudTextId::new(&trust.raw_hub_id).map_err(remote)?;
        let namespace = super::metadata_crypto::namespace(&binding).map_err(remote)?;
        if hub_namespace_id(
            raw_hub_id.as_str(),
            *binding.scope().hub().authority().as_bytes(),
        )
        .map_err(remote)?
            != namespace.hub_instance_id
        {
            return Err(RunnerHubError::Binding);
        }
        let subject = RunnerProofSubject {
            raw_hub_id,
            namespace,
            runner_id: *binding.runner().as_bytes(),
            proof_key: *public.proof_key(),
        };
        let endpoint = crate::endpoint::endpoint(&trust.endpoint).map_err(remote)?;
        let file = RunnerSessionFile::acquire(private_directory, &binding, &subject)?;
        Ok(Arc::new(Self {
            endpoint,
            binding,
            identity,
            subject,
            authority: trust.authority,
            work,
            session: Mutex::default(),
            file: Mutex::new(file),
            notice: tokio::sync::watch::channel(()).0,
            operation: tokio::sync::Mutex::new(()),
            retired: AtomicBool::new(false),
        }))
    }

    pub fn binding(&self) -> &JournalCatalogBinding {
        &self.binding
    }
    pub(super) fn shares_identity(&self, identity: &Arc<CloudRunnerIdentity>) -> bool {
        Arc::ptr_eq(&self.identity, identity)
    }

    pub fn check_source(&self) -> Result<(), RunnerHubError> {
        self.runner().map(|_| ())
    }

    /// The original runtime survives known credential rotation/persistence. This
    /// check is not RPC admission or a fresh authenticated head witness.
    pub fn check_runtime_source(&self) -> Result<(), RunnerHubError> {
        self.check_identity()?;
        self.session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .runtime(runtime::now())?;
        self.check_identity()
    }
    /// False is temporary transport readiness only, never authenticated authority.
    pub fn transport_ready(&self) -> Result<bool, RunnerHubError> {
        self.check_identity()?;
        let ready = self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .transport_ready(runtime::now())?;
        self.check_identity()?;
        Ok(ready)
    }
    /// No payload reservation while waiting. Actual retirement/phase changes wake
    /// waiters; finite clock polls also enforce the accepted hard deadline.
    pub fn wait_transport_ready(
        self: &Arc<Self>,
    ) -> futures::future::BoxFuture<'static, Result<(), RunnerHubError>> {
        let owner = self.clone();
        Box::pin(async move {
            let mut notice = owner.notice.subscribe();
            loop {
                if owner.transport_ready()? {
                    return Ok(());
                }
                let interval = owner.wait_interval()?;
                tokio::select! {
                    changed=notice.changed()=> { changed.map_err(|_| RunnerHubError::Unavailable)?; },
                    _=tokio::time::sleep(interval)=> {},
                }
            }
        })
    }
    pub(in crate::cloud) async fn capture_transport(
        self: &Arc<Self>,
    ) -> Result<CapturedTransport, RunnerHubError> {
        loop {
            self.wait_transport_ready().await?;
            self.check_identity()?;
            let captured = self
                .session
                .lock()
                .map_err(|_| RunnerHubError::Unavailable)?
                .capture(runtime::now());
            match captured {
                Ok((runner, generation)) => {
                    return Ok(CapturedTransport::new(self.clone(), runner, generation));
                }
                Err(RunnerHubError::DurabilityPending) => {}
                Err(error) => return Err(error),
            }
        }
    }
    fn changed(&self) {
        self.notice.send_replace(());
    }
    fn wait_interval(&self) -> Result<std::time::Duration, RunnerHubError> {
        self.check_identity()?;
        let millis = self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .wait_millis(runtime::now())?;
        Ok(std::time::Duration::from_millis(millis))
    }
    pub fn credential_deadlines(&self) -> Result<RunnerCredentialDeadlines, RunnerHubError> {
        self.check_identity()?;
        self.session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .deadlines(runtime::now())
    }

    pub async fn redeem(
        self: &Arc<Self>,
        ticket: Zeroizing<String>,
    ) -> Result<RunnerHubPeer, RunnerHubError> {
        let _turn = self.operation.lock().await;
        self.check_identity()?;
        if self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .has_runner()
        {
            return Err(RunnerHubError::Existing);
        }
        self.file
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .require_absent()?;
        let response = CloudRunner::redeem(
            &self.endpoint,
            self.subject.clone(),
            self.authority,
            self.proof()?,
            ticket,
        )
        .await;
        self.check_identity()?;
        let runner = response.map_err(sdk_error)?;
        self.install(runner, true)?;
        self.peer()
    }

    /// Restore never rotates remotely. Expired access requires an explicit
    /// refresh; a still-live refresh credential is sufficient to restore.
    pub async fn restore(self: &Arc<Self>) -> Result<RunnerHubPeer, RunnerHubError> {
        let _turn = self.operation.lock().await;
        self.check_identity()?;
        if self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .has_runner()
        {
            return Err(RunnerHubError::Existing);
        }
        let session = self
            .file
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .read(&self.subject)?;
        let response = CloudRunner::restore(
            &self.endpoint,
            self.subject.clone(),
            self.authority,
            self.proof()?,
            session,
        )
        .await;
        self.check_identity()?;
        let runner = response.map_err(sdk_error)?;
        self.install(runner, false)?;
        self.peer()
    }

    fn proof(&self) -> Result<DeviceKey, RunnerHubError> {
        self.identity
            .with_keys(|keys| DeviceKey::from_seed(&keys.proof().seed()))
            .map_err(Into::into)
    }

    fn record(
        &self,
        session: nocterm_hub_client::cloud::RunnerSession,
    ) -> Result<Arc<SessionRecord>, RunnerHubError> {
        let file = self.file.lock().map_err(|_| RunnerHubError::Unavailable)?;
        Ok(Arc::new(SessionRecord::from_session(session, &file)?))
    }
    fn install(&self, runner: CloudRunner, persist: bool) -> Result<(), RunnerHubError> {
        if let Err(error) = self.check_identity() {
            runner.revoke_local();
            return Err(error);
        }
        let pending = if persist {
            Some(self.record(runner.session().map_err(sdk_error)?)?)
        } else {
            None
        };
        {
            let mut state = self
                .session
                .lock()
                .map_err(|_| RunnerHubError::Unavailable)?;
            if let Err(error) = self.check_identity() {
                runner.revoke_local();
                return Err(error);
            }
            state.install(runner, pending, runtime::now())?;
        }
        self.changed();
        self.persist_pending()?;
        self.check_identity()
    }
    fn persist_pending(&self) -> Result<(), RunnerHubError> {
        let pending = self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .pending();
        let Some(pending) = pending else {
            return Ok(());
        };
        self.check_identity()?;
        // Slow private disk publication never holds the runtime authority lock.
        self.file
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .save(&pending)?;
        self.check_identity()?;
        if !self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .persisted(&pending)
        {
            return Err(RunnerHubError::Unavailable);
        }
        self.changed();
        Ok(())
    }
    /// Retry the exact known returned session's disk publication, never another RPC.
    pub async fn retry_persist(self: &Arc<Self>) -> Result<RunnerHubPeer, RunnerHubError> {
        let _turn = self.operation.lock().await;
        self.check_identity()?;
        self.persist_pending()?;
        self.peer()
    }
    /// Known active refresh is separate from transport readiness. Cancellation,
    /// failure or unobserved outcome permanently fences this original runtime.
    pub async fn refresh(self: &Arc<Self>) -> Result<RunnerHubPeer, RunnerHubError> {
        let _turn = self.operation.lock().await;
        self.check_identity()?;
        let (runner, token) = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| RunnerHubError::Unavailable)?;
            self.check_identity()?;
            state.begin(runtime::now())?
        };
        let guard = RefreshGuard::new(self.clone(), token);
        self.changed();
        let session = runner.refresh().await.map_err(sdk_error)?;
        self.check_identity()?;
        let deadlines = RunnerCredentialDeadlines {
            access: session.access_expires_at_ms(),
            refresh: session.refresh_expires_at_ms(),
        };
        let record = self.record(session)?;
        self.session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .returned(guard.token(), record, deadlines, runtime::now())?;
        self.changed();
        self.persist_pending()?;
        self.peer()
    }

    pub fn peer(self: &Arc<Self>) -> Result<RunnerHubPeer, RunnerHubError> {
        self.runner()?;
        Ok(RunnerHubPeer {
            connection: self.clone(),
        })
    }

    /// Retire this runtime and its clones. Persistent credentials remain available
    /// for explicit restart; account-issued remote revocation is separate.
    /// Closing a desktop observer must not call this method.
    pub fn retire_local(&self) {
        self.retired.store(true, Ordering::Release);
        let mut state = self
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.retire();
        drop(state);
        self.changed();
    }

    pub(super) fn check_identity(&self) -> Result<(), RunnerHubError> {
        if self.retired.load(Ordering::Acquire) {
            return Err(RunnerHubError::Unavailable);
        }
        self.identity.with_keys(|_| ())?;
        if self.retired.load(Ordering::Acquire) {
            return Err(RunnerHubError::Unavailable);
        }
        Ok(())
    }
    pub(super) fn runner(&self) -> Result<CloudRunner, RunnerHubError> {
        self.check_identity()?;
        let runner = self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?
            .ready(runtime::now())?;
        self.check_identity()?;
        Ok(runner)
    }
    pub(super) fn work(&self) -> &Arc<dyn PublicationWorkBudget> {
        &self.work
    }
}

impl Drop for RunnerHubConnection {
    fn drop(&mut self) {
        self.retire_local();
    }
}

#[derive(Clone)]
pub struct RunnerHubPeer {
    connection: Arc<RunnerHubConnection>,
}
impl RunnerHubPeer {
    pub fn binding(&self) -> &JournalCatalogBinding {
        self.connection.binding()
    }
    pub(super) fn connection(&self) -> &Arc<RunnerHubConnection> {
        &self.connection
    }
}

pub(super) fn remote(error: impl std::error::Error + Send + Sync + 'static) -> RunnerHubError {
    RunnerHubError::Remote(SharedCause::new(error))
}

pub(super) fn sdk_error(error: CloudError) -> RunnerHubError {
    match error {
        error @ (CloudError::Unsupported
        | CloudError::Capability(
            nocterm_hub_model::cloud::capabilities::CapabilityError::Unsupported,
        )) => RunnerHubError::Unsupported(SharedCause::new(error)),
        error @ (CloudError::SourceChanged | CloudError::NotSignedIn) => {
            RunnerHubError::SourceChanged(SharedCause::new(error))
        }
        error => remote(error),
    }
}

#[cfg(test)]
mod tests;
