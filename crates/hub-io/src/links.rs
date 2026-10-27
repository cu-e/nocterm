//! The hub adapter's entry point: the runtime and the three ports.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::FutureExt as _;
use futures::channel::oneshot;
use nocterm_hub_client::HubEndpoint;
use nocterm_hubs_model::{
    Account, Fingerprint, HubAddress, HubEndpointSpec, HubError, HubFuture, HubId, HubInfo,
    HubSessionId, HubSessionSummary, HubSessions, HubSignIn, HubTerminal, HubTerminalTarget,
    HubTrust, KeyStatus, NodeSummary, OidcSignIn, OrgId, ProbedHub, SignedIn,
};
use nocterm_session::Secret;
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::AbortHandle;

use crate::account::{Accounts, KeyFlow};
use crate::classify::{probe_failed, remote};
use crate::connect::Connector;
use crate::directory::Directory;
use crate::hub_account::HubAccount;
use crate::pool::Pool;
use crate::terminal::HubTerminalTransport;

/// Threads serving every hub connection of the application.
const WORKER_THREADS: usize = 2;

/// Where the adapter keeps what it needs and how this device is named to hubs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HubLinksConfig {
    /// The private directory with one credential file per hub. Created when first needed.
    pub credentials_dir: PathBuf,
    /// The machine as the user knows it, listed by every hub this device signs in to.
    pub device_name: String,
}

/// The runtime of the adapter could not start.
#[derive(Debug, thiserror::Error)]
#[error("the hub connection threads cannot start")]
pub struct HubStartError(#[source] std::io::Error);

/// Reaches hubs. Implements the hub ports of `nocterm-hubs-model`.
///
/// Calls run on the adapter's threads; dropping the future of a call abandons it. Dropping the
/// links ends every call under way.
pub struct HubLinks {
    // `None` only while the links are being dropped.
    runtime: Option<Runtime>,
    handle: Handle,
    pool: Arc<Pool>,
    connector: Arc<Connector>,
    directory: Arc<Directory>,
    accounts: Arc<Accounts>,
    keys: Arc<KeyFlow>,
}

/// The adapter's threads have stopped.
#[derive(Debug, thiserror::Error)]
#[error("the hub connection threads have stopped")]
struct Stopped;

/// The result of a task on the adapter's runtime; abandons the task when dropped.
struct Call<T> {
    result: oneshot::Receiver<Result<T, HubError>>,
    task: Option<AbortHandle>,
}

impl<T> Future for Call<T> {
    type Output = Result<T, HubError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.result
            .poll_unpin(cx)
            .map(|received| received.unwrap_or_else(|_| Err(remote(Stopped))))
    }
}

impl<T> Drop for Call<T> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl HubLinks {
    pub fn start(config: HubLinksConfig) -> Result<Arc<Self>, HubStartError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("nocterm-hub")
            .enable_all()
            .build()
            .map_err(HubStartError)?;
        let accounts = Arc::new(Accounts::new(config.credentials_dir.clone()));
        let pool = Arc::new(Pool::new(
            config.credentials_dir,
            &config.device_name,
            Arc::clone(&accounts),
        ));
        let keys = Arc::new(KeyFlow::new(Arc::clone(&pool), Arc::clone(&accounts)));
        Ok(Arc::new(Self {
            handle: runtime.handle().clone(),
            runtime: Some(runtime),
            connector: Arc::new(Connector::new(
                Arc::clone(&pool),
                Arc::clone(&accounts),
                Arc::clone(&keys),
            )),
            directory: Arc::new(Directory::new(Arc::clone(&pool))),
            pool,
            accounts,
            keys,
        }))
    }

    /// The encrypted data of the account this device is signed in to on `hub`: its profile
    /// replica and its vault. Both share the account's keys, which open at sign-in and close
    /// when the vault locks or the device signs out.
    pub fn open_account(&self, hub: &HubId) -> Result<HubAccount, HubError> {
        let slot = self.accounts.slot(hub)?;
        Ok(HubAccount::new(
            self.handle.clone(),
            Arc::clone(&self.pool),
            slot,
        ))
    }

    /// What a relay needs of the links: their threads and the hubs they are signed in to.
    pub(crate) fn relay_parts(&self) -> (Handle, Arc<Pool>) {
        (self.handle.clone(), Arc::clone(&self.pool))
    }

    /// Runs `work` on the adapter's threads.
    fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, HubError>> + Send + 'static,
    ) -> Call<T> {
        let (sender, result) = oneshot::channel();
        let task = self
            .handle
            .spawn(async move {
                let _ = sender.send(work.await);
            })
            .abort_handle();
        Call {
            result,
            task: Some(task),
        }
    }

    fn run_boxed<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, HubError>> + Send + 'static,
    ) -> HubFuture<T> {
        Box::pin(self.run(work))
    }
}

impl Drop for HubLinks {
    fn drop(&mut self) {
        // A runtime must not block where it is dropped: that may be a thread that is itself
        // running async code.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl HubTrust for HubLinks {
    fn probe(&self, address: &HubAddress) -> HubFuture<ProbedHub> {
        let address = address.clone();
        self.run_boxed(async move {
            let probe = HubEndpoint::probe(&address.to_string())
                .await
                .map_err(|error| probe_failed(&address, error))?;
            Ok(ProbedHub {
                fingerprint: Fingerprint::from_bytes(probe.fingerprint),
                publicly_trusted: probe.publicly_trusted,
            })
        })
    }
}

impl HubSignIn for HubLinks {
    fn connect(&self, endpoint: HubEndpointSpec) -> HubFuture<HubInfo> {
        let connector = Arc::clone(&self.connector);
        self.run_boxed(async move { connector.connect(endpoint).await })
    }

    fn password(
        &self,
        endpoint: HubEndpointSpec,
        username: String,
        password: Secret,
    ) -> HubFuture<SignedIn> {
        let connector = Arc::clone(&self.connector);
        self.run_boxed(async move { connector.password(endpoint, username, password).await })
    }

    fn oidc(&self, endpoint: HubEndpointSpec, provider: String) -> OidcSignIn {
        let connector = Arc::clone(&self.connector);
        let (browser_sender, browser) = oneshot::channel();
        let done =
            self.run_boxed(async move { connector.oidc(endpoint, provider, browser_sender).await });
        OidcSignIn { browser, done }
    }

    fn set_vault_passphrase(&self, hub: &HubId, passphrase: Secret) -> HubFuture<KeyStatus> {
        let (keys, hub) = (Arc::clone(&self.keys), hub.clone());
        self.run_boxed(async move { keys.passphrase(&hub, passphrase).await })
    }

    fn recover(&self, hub: &HubId, recovery_key: Secret) -> HubFuture<()> {
        let (keys, hub) = (Arc::clone(&self.keys), hub.clone());
        self.run_boxed(async move { keys.recover(&hub, recovery_key).await })
    }

    fn recovery_key_unconfirmed(&self, hub: &HubId) -> bool {
        self.keys.recovery_key_unconfirmed(hub)
    }

    fn reissue_recovery_key(&self, hub: &HubId) -> HubFuture<Secret> {
        let (keys, hub) = (Arc::clone(&self.keys), hub.clone());
        self.run_boxed(async move { keys.reissue_recovery_key(&hub).await })
    }

    fn confirm_recovery_key(&self, hub: &HubId) -> Result<(), HubError> {
        self.keys.confirm_recovery_key(hub)
    }

    fn sign_out(&self, hub: &HubId) -> HubFuture<()> {
        let (connector, hub) = (Arc::clone(&self.connector), hub.clone());
        self.run_boxed(async move { connector.sign_out(&hub).await })
    }

    fn forget(&self, hub: &HubId) -> Result<(), HubError> {
        self.connector.forget(hub)
    }
}

impl HubSessions for HubLinks {
    fn account(&self, hub: &HubId) -> HubFuture<Account> {
        let (directory, hub) = (Arc::clone(&self.directory), hub.clone());
        self.run_boxed(async move { directory.account(&hub).await })
    }

    fn nodes(&self, hub: &HubId, org: &OrgId) -> HubFuture<Vec<NodeSummary>> {
        let (directory, hub, org) = (Arc::clone(&self.directory), hub.clone(), org.clone());
        self.run_boxed(async move { directory.nodes(&hub, &org).await })
    }

    fn sessions(&self, hub: &HubId, org: &OrgId) -> HubFuture<Vec<HubSessionSummary>> {
        let (directory, hub, org) = (Arc::clone(&self.directory), hub.clone(), org.clone());
        self.run_boxed(async move { directory.sessions(&hub, &org).await })
    }

    fn end(&self, hub: &HubId, session: &HubSessionId) -> HubFuture<()> {
        let (directory, hub, session) = (Arc::clone(&self.directory), hub.clone(), session.clone());
        self.run_boxed(async move { directory.end(&hub, &session).await })
    }

    fn terminal(&self, hub: &HubId, target: HubTerminalTarget) -> HubTerminal {
        let (transport, created) = HubTerminalTransport::new(
            self.handle.clone(),
            Arc::clone(&self.pool),
            hub.clone(),
            target,
        );
        match created {
            Some(created) => HubTerminal::starting(Arc::new(transport), created),
            None => HubTerminal::attached(Arc::new(transport)),
        }
    }
}
