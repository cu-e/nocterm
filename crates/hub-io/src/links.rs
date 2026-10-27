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
    HubSessionId, HubSessionSummary, HubSessions, HubSignIn, HubTrust, NodeSummary, OidcSignIn,
    OrgId, ProbedHub,
};
use nocterm_session::Secret;
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::task::AbortHandle;

use crate::classify::{probe_failed, remote};
use crate::connect::Connector;
use crate::directory::Directory;
use crate::pool::Pool;

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
    connector: Arc<Connector>,
    directory: Arc<Directory>,
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
        let pool = Arc::new(Pool::new(config.credentials_dir, &config.device_name));
        Ok(Arc::new(Self {
            runtime: Some(runtime),
            connector: Arc::new(Connector::new(Arc::clone(&pool))),
            directory: Arc::new(Directory::new(pool)),
        }))
    }

    fn handle(&self) -> Option<&Handle> {
        self.runtime.as_ref().map(Runtime::handle)
    }

    /// Runs `work` on the adapter's threads.
    fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, HubError>> + Send + 'static,
    ) -> Call<T> {
        let (sender, result) = oneshot::channel();
        let task = self.handle().map(|handle| {
            handle
                .spawn(async move {
                    let _ = sender.send(work.await);
                })
                .abort_handle()
        });
        Call { result, task }
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
    ) -> HubFuture<Account> {
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
}
