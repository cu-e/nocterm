//! The hub adapter's entry point: the runtime and the three ports.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use nocterm_hub_client::HubEndpoint;
use nocterm_hubs_model::{
    AccountDevices, DeviceSummary, Fingerprint, HubAddress, HubEndpointSpec, HubError, HubFuture,
    HubId, HubInfo, HubSignIn, HubTrust, KeyStatus, OidcSignIn, ProbedHub, SignedIn,
};
use nocterm_session::Secret;
use tokio::runtime::{Builder, Handle, Runtime};

use crate::account::{Accounts, KeyFlow};
use crate::classify::{probe_failed, remote};
use crate::connect::Connector;
use crate::hub_account::HubAccount;
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
    handle: Handle,
    pool: Arc<Pool>,
    connector: Arc<Connector>,
    accounts: Arc<Accounts>,
    keys: Arc<KeyFlow>,
    chat_hints: Arc<crate::chats::ChatHints>,
}

/// The adapter's threads have stopped.
#[derive(Debug, thiserror::Error)]
#[error("the hub connection threads have stopped")]
struct Stopped;

impl HubLinks {
    pub fn start(config: HubLinksConfig) -> Result<Arc<Self>, HubStartError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("nocterm-hub")
            .enable_all()
            .build()
            .map_err(HubStartError)?;
        let accounts = Arc::new(Accounts::with_runtime(
            config.credentials_dir.clone(),
            Some(runtime.handle().clone()),
        ));
        let pool = Arc::new(Pool::new(
            config.credentials_dir,
            &config.device_name,
            Arc::clone(&accounts),
        ));
        let keys = KeyFlow::new(
            Arc::clone(&pool),
            Arc::clone(&accounts),
            runtime.handle().clone(),
        );
        Ok(Arc::new(Self {
            handle: runtime.handle().clone(),
            runtime: Some(runtime),
            connector: Arc::new(Connector::new(Arc::clone(&pool), Arc::clone(&keys))),
            pool,
            accounts,
            keys,
            chat_hints: Arc::default(),
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

    pub(crate) fn account_keys_open(&self, hub: &HubId) -> bool {
        self.accounts
            .loaded_slot(hub)
            .is_some_and(|slot| slot.open().is_some())
    }

    /// Native membership publication always creates its Slot first. An absent
    /// loaded Slot therefore cannot be unlocked for a current native recipient.
    pub(crate) fn loaded_account(&self, hub: &HubId) -> Option<HubAccount> {
        self.accounts
            .loaded_slot(hub)
            .map(|slot| HubAccount::new(self.handle.clone(), self.pool.clone(), slot))
    }

    /// What a relay needs of the links: their threads and the hubs they are signed in to.
    pub(crate) fn relay_parts(&self) -> (Handle, Arc<Pool>) {
        (self.handle.clone(), Arc::clone(&self.pool))
    }

    /// Hints that a chat list kept on this device changed.
    pub(crate) fn chat_hints(&self) -> Arc<crate::chats::ChatHints> {
        Arc::clone(&self.chat_hints)
    }

    /// What the agents and chats adapters need of the links.
    pub(crate) fn agent_parts(&self) -> (Handle, Arc<Pool>) {
        (self.handle.clone(), Arc::clone(&self.pool))
    }

    fn run_boxed<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, HubError>> + Send + 'static,
    ) -> HubFuture<T> {
        crate::task::spawn(&self.handle, work)
            .map(|result| result.unwrap_or_else(|| Err(remote(Stopped))))
            .boxed()
    }
}

impl Drop for HubLinks {
    fn drop(&mut self) {
        self.pool.stop();
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
        match self.connector.reconnect(endpoint) {
            Ok(request) => request.start(&self.handle),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn password(
        &self,
        endpoint: HubEndpointSpec,
        username: String,
        password: Secret,
    ) -> HubFuture<SignedIn> {
        let connector = Arc::clone(&self.connector);
        match connector.begin(endpoint) {
            Ok(start) => start.start(&self.handle, move |start| async move {
                connector.password(start, username, password).await
            }),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn oidc(&self, endpoint: HubEndpointSpec, provider: String) -> OidcSignIn {
        let connector = Arc::clone(&self.connector);
        let (browser_sender, browser_receiver) = oneshot::channel();
        match connector.begin(endpoint) {
            Ok(start) => {
                let observed = start.observation().clone();
                let done = start.start(&self.handle, move |start| async move {
                    connector.oidc(start, provider, browser_sender).await
                });
                let browser = Box::pin(async move {
                    observed.check()?;
                    let result = observed
                        .during(browser_receiver)
                        .await?
                        .map_err(|_| HubError::Stale)?;
                    observed.check()?;
                    Ok(result)
                });
                OidcSignIn { browser, done }
            }
            Err(error) => OidcSignIn {
                browser: Box::pin(async { Err(HubError::Stale) }),
                done: Box::pin(async move { Err(error) }),
            },
        }
    }

    fn set_vault_passphrase(&self, hub: &HubId, passphrase: Secret) -> HubFuture<KeyStatus> {
        let keys = self.keys.clone();
        self.keys
            .run(self.keys.capture(hub), move |owner| async move {
                keys.passphrase(&owner, passphrase).await
            })
    }

    fn recover(&self, hub: &HubId, recovery_key: Secret) -> HubFuture<()> {
        let keys = self.keys.clone();
        self.keys
            .run(self.keys.capture(hub), move |owner| async move {
                keys.recover(&owner, recovery_key).await
            })
    }

    fn recovery_key_unconfirmed(&self, hub: &HubId) -> bool {
        self.keys.recovery_key_unconfirmed(hub)
    }

    fn keys_revision(&self) -> u64 {
        self.keys.keys_revision()
    }

    fn keys_changed(&self, seen: u64) -> BoxFuture<'static, u64> {
        self.keys.keys_changed(seen)
    }

    fn recovery_key_offer(
        &self,
        hub: &HubId,
    ) -> Result<nocterm_hubs_model::RecoveryKeyReissue, HubError> {
        self.keys.recovery_offer(hub)
    }

    fn sign_out(&self, hub: &HubId) -> HubFuture<()> {
        match self.connector.sign_out(hub) {
            Ok(request) => request.start(&self.handle),
            Err(error) => Box::pin(async move { Err(error) }),
        }
    }

    fn forget(&self, hub: &HubId) -> Result<(), HubError> {
        self.connector.forget(hub)
    }
}

impl AccountDevices for HubLinks {
    fn list(&self, hub: &HubId) -> HubFuture<Vec<DeviceSummary>> {
        crate::devices::list(Arc::clone(&self.pool), &self.handle, hub)
    }

    fn revoke(&self, hub: &HubId, device: &str) -> HubFuture<()> {
        crate::devices::revoke(Arc::clone(&self.pool), &self.handle, hub, device)
    }
}
