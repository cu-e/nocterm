//! Independent runner credentials; desktop membership never owns this lifetime.
use super::{CloudRunnerIdentity, CloudRunnerIdentityError};
use nocterm_cloud_model::journal::{JournalCatalogBinding, PublicationWorkBudget};
use nocterm_core::SharedCause;
use nocterm_hub_client::{
    HubEndpoint,
    cloud::{CloudError, CloudRunner},
};
use nocterm_hub_model::cloud::{CloudTextId, hub_namespace_id, runner_auth::RunnerProofSubject};
use nocterm_hubs_model::HubEndpointSpec;
use nocterm_keys::DeviceKey;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
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
}

struct SessionOwner {
    file: RunnerSessionFile,
    runner: Option<CloudRunner>,
    pending: Option<SessionRecord>,
    rotating: bool,
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
            session: Mutex::new(SessionOwner {
                file,
                runner: None,
                pending: None,
                rotating: false,
            }),
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

    pub async fn redeem(
        self: &Arc<Self>,
        ticket: Zeroizing<String>,
    ) -> Result<RunnerHubPeer, RunnerHubError> {
        let _turn = self.operation.lock().await;
        self.check_identity()?;
        {
            let state = self
                .session
                .lock()
                .map_err(|_| RunnerHubError::Unavailable)?;
            if state.runner.is_some() {
                return Err(RunnerHubError::Existing);
            }
            state.file.require_absent()?;
        }
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
        let session = {
            let mut state = self
                .session
                .lock()
                .map_err(|_| RunnerHubError::Unavailable)?;
            if state.runner.is_some() {
                return Err(RunnerHubError::Existing);
            }
            state.file.read(&self.subject)?
        };
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

    fn install(&self, runner: CloudRunner, persist: bool) -> Result<(), RunnerHubError> {
        let mut state = self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?;
        if let Err(error) = self.check_identity() {
            runner.revoke_local();
            return Err(error);
        }
        let pending = if persist {
            Some(SessionRecord::from_session(
                runner.session().map_err(sdk_error)?,
                &state.file,
            )?)
        } else {
            None
        };
        state.runner = Some(runner);
        state.pending = pending;
        persist_pending(&mut state)?;
        self.check_identity()
    }

    /// Retry the exact successful rotation's native write, without another RPC.
    pub async fn retry_persist(self: &Arc<Self>) -> Result<RunnerHubPeer, RunnerHubError> {
        let _turn = self.operation.lock().await;
        self.check_identity()?;
        {
            let mut state = self
                .session
                .lock()
                .map_err(|_| RunnerHubError::Unavailable)?;
            persist_pending(&mut state)?;
        }
        self.peer()
    }

    /// Close admission before remote rotation. An unknown/cancelled RPC result
    /// stays fenced; only a known returned session can use `retry_persist`.
    pub async fn refresh(self: &Arc<Self>) -> Result<RunnerHubPeer, RunnerHubError> {
        let _turn = self.operation.lock().await;
        let runner = self.runner()?;
        {
            let mut state = self
                .session
                .lock()
                .map_err(|_| RunnerHubError::Unavailable)?;
            // Admission closes before the SDK can rotate its shared credentials.
            // Cancellation or an unknown remote refresh result stays fail closed;
            // it must never cause an automatic second rotation request.
            state.rotating = true;
        }
        let response = runner.refresh().await;
        self.check_identity()?;
        let session = response.map_err(sdk_error)?;
        let mut state = self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?;
        self.check_identity()?;
        state.pending = Some(SessionRecord::from_session(session, &state.file)?);
        persist_pending(&mut state)?;
        drop(state);
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
        if let Some(runner) = state.runner.take() {
            runner.revoke_local();
        }
        state.pending.take();
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
        let state = self
            .session
            .lock()
            .map_err(|_| RunnerHubError::Unavailable)?;
        if state.rotating || state.pending.is_some() {
            return Err(RunnerHubError::DurabilityPending);
        }
        let runner = state.runner.as_ref().ok_or(RunnerHubError::Unavailable)?;
        runner.check_source().map_err(sdk_error)?;
        self.check_identity()?;
        Ok(runner.clone())
    }
    pub(super) fn work(&self) -> &Arc<dyn PublicationWorkBudget> {
        &self.work
    }
}

fn persist_pending(state: &mut SessionOwner) -> Result<(), RunnerHubError> {
    if let Some(record) = state.pending.as_ref() {
        state.file.save(record)?;
        state.pending.take();
        state.rotating = false;
    }
    Ok(())
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
