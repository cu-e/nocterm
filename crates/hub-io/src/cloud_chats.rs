//! The Cloud chat port of `nocterm-cloud-agents-model`, over the contract's device core.
//!
//! Every call captures the signed-in membership before it is spawned and rechecks it when it
//! ends, as [`crate::agents`] does. The account's Cloud session (hub trust, verified
//! capabilities, trusted runners) is discovered by the core's `Discover` flow and shared while
//! a chat of it is open. Listing runs `ListChats` over the runners' directories and catalogs;
//! creating runs `CreateChat` (ADR 0122); a chat opens as a [`DeviceConversation`] over
//! `OpenChat`, which wakes a managed seat before each command (ADR 0133). An own-agent chat's
//! provider key is leased by `ProviderLease` while the chat is open (ADR 0126).

mod create;
mod directory;
mod error;
mod leases;
mod managed;

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use nocterm_cloud_agents_model::{
    AgentId, ChatFuture, ChatHistoryEntry, ChatHost, ChatRunner, CloudChatError, CloudChatLauncher,
    CloudFuture, OpenedChat, OwnRunnerLauncher, OwnRunnerSeat, StepSink,
};
use nocterm_hubs_model::HubId;
use tokio::runtime::Handle;

use crate::cloud_device::{ChatSetup, Checkpoints, DeviceAccount, DeviceConversation, Outbox};
use crate::devices::Captured;
use crate::hub_account::HubAccount;
use crate::links::HubLinks;
use crate::pool::Pool;
use crate::task::spawn;

/// Where the installation keeps its Cloud chat files, injected by the application.
pub struct ChatServices {
    root: PathBuf,
}

impl ChatServices {
    /// The services of an application over `links`: sealed chat checkpoints under `root`. The
    /// verified-page caches of the earlier layout there are removed.
    pub fn native(_links: &HubLinks, root: PathBuf) -> Result<Self, CloudChatError> {
        crate::cloud_device::remove_earlier_pages(&root);
        Ok(Self { root })
    }
}

/// What one operation shares with the others of the installation.
struct Shared {
    handle: Handle,
    pool: Arc<Pool>,
    links: Arc<HubLinks>,
    checkpoints: Arc<Checkpoints>,
    outbox: Arc<Outbox>,
    /// The open Cloud sessions per hub, while a chat of them holds one.
    accounts: Mutex<HashMap<HubId, Weak<DeviceAccount>>>,
    leases: leases::Leases,
}

impl Shared {
    /// Captures the signed-in membership and account of `hub`, then runs `work` with them.
    fn call<T, F, Fut>(self: &Arc<Self>, hub: &HubId, work: F) -> ChatFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Shared>, Arc<Captured>, HubAccount) -> Fut,
        Fut: Future<Output = Result<T, CloudChatError>> + Send + 'static,
    {
        let prepared = (|| {
            let captured = Arc::new(Captured::new(self.pool.clone(), hub).map_err(error::hub)?);
            let account = self.links.open_account(hub).map_err(error::hub)?;
            Ok::<_, CloudChatError>((captured, account))
        })();
        match prepared {
            Ok((captured, account)) => {
                let work = work(self.clone(), captured.clone(), account);
                self.run(&captured, work)
            }
            Err(failure) => Box::pin(async move { Err(failure) }),
        }
    }

    /// Runs `work` on the adapter's threads and checks the captured membership again when it
    /// ends: a result for an account that was signed out or replaced meanwhile is `Stale`.
    fn run<T, Fut>(&self, captured: &Arc<Captured>, work: Fut) -> ChatFuture<T>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T, CloudChatError>> + Send + 'static,
    {
        let observed = captured.observed.clone();
        let task = spawn(&self.handle, work);
        Box::pin(async move {
            let result = task.await.unwrap_or(Err(CloudChatError::Stale));
            observed.check().map_err(error::hub)?;
            result
        })
    }

    /// The account's Cloud session: the one an open chat holds, or a new one.
    async fn account(
        &self,
        captured: Arc<Captured>,
        account: &HubAccount,
    ) -> Result<Arc<DeviceAccount>, CloudChatError> {
        let held = self
            .accounts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&captured.hub)
            .and_then(Weak::upgrade)
            .filter(|held| held.check().is_ok());
        if let Some(held) = held {
            return Ok(held);
        }
        let hub = captured.hub.clone();
        let opened = DeviceAccount::open(&self.pool, captured, account)
            .await
            .map_err(error::account)?;
        self.accounts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(hub, Arc::downgrade(&opened));
        Ok(opened)
    }

    /// A chat of `account` as a conversation a window presents.
    fn conversation(
        &self,
        account: Arc<DeviceAccount>,
        setup: ChatSetup,
    ) -> Arc<DeviceConversation> {
        Arc::new(DeviceConversation::new(
            account,
            setup,
            (self.checkpoints.clone(), self.outbox.clone()),
            self.handle.clone(),
        ))
    }
}

/// Reaches the Cloud chats of the hubs this device is signed in to.
pub struct CloudChatsLink {
    shared: Arc<Shared>,
}

impl CloudChatsLink {
    /// Fails with `Capacity` when the installation cannot be set up.
    pub fn new(links: Arc<HubLinks>, services: ChatServices) -> Result<Self, CloudChatError> {
        let (handle, pool) = links.agent_parts();
        let shared = Arc::new(Shared {
            handle: handle.clone(),
            pool: pool.clone(),
            links,
            checkpoints: Arc::new(Checkpoints::new(services.root.clone())),
            outbox: Arc::new(Outbox::new(services.root)),
            accounts: Mutex::new(HashMap::new()),
            leases: leases::Leases::default(),
        });
        Ok(Self { shared })
    }

    fn call<T, F, Fut>(&self, hub: &HubId, work: F) -> ChatFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Shared>, Arc<Captured>, HubAccount) -> Fut,
        Fut: Future<Output = Result<T, CloudChatError>> + Send + 'static,
    {
        self.shared.call(hub, work)
    }
}

/// The person's own runners of `account`: registered, and not the seat of a managed agent.
async fn own_runners(
    captured: &Arc<Captured>,
    account: &DeviceAccount,
) -> Result<Vec<ChatRunner>, CloudChatError> {
    let seats = managed::seats(captured).await?;
    Ok(account
        .session()
        .runners()
        .iter()
        .filter(|row| managed::is_own_runner(row.label(), &row.runner().id, &seats))
        .map(|row| ChatRunner {
            id: row.runner().id,
            label: row.label().to_owned(),
        })
        .collect())
}

impl CloudChatLauncher for CloudChatsLink {
    fn runners(&self, hub: &HubId) -> ChatFuture<Vec<ChatRunner>> {
        self.call(hub, |shared, captured, account| async move {
            let opened = shared.account(captured.clone(), &account).await?;
            own_runners(&captured, &opened).await
        })
    }

    fn create(&self, hub: &HubId, host: ChatHost) -> ChatFuture<OpenedChat> {
        let hub_id = hub.clone();
        self.call(hub, move |shared, captured, account| async move {
            create::create(&shared, captured, &account, &hub_id, host).await
        })
    }

    fn history(&self, hub: &HubId) -> ChatFuture<Vec<Arc<dyn ChatHistoryEntry>>> {
        let hub_id = hub.clone();
        self.call(hub, move |shared, captured, account| async move {
            directory::list(&shared, captured, &account, hub_id).await
        })
    }
}

impl OwnRunnerLauncher for CloudChatsLink {
    fn start(&self, hub: &HubId, agent: &AgentId, _steps: StepSink) -> CloudFuture<OwnRunnerSeat> {
        let (hub_id, agent) = (hub.clone(), agent.clone());
        let call = self.call(hub, move |shared, captured, account| async move {
            let opened = shared.account(captured.clone(), &account).await?;
            let runner = own_runners(&captured, &opened)
                .await?
                .first()
                .map(|runner| runner.id)
                .ok_or(CloudChatError::NotEnrolled)?;
            leases::key_model(&account, &agent).await?;
            shared.leases.track(&hub_id, &agent, runner);
            Ok(OwnRunnerSeat {
                runner: runner.to_vec(),
                lease_ttl: leases::LEASE_TERM,
            })
        });
        Box::pin(async move { call.await.map_err(leases::agent_error) })
    }

    fn renew(&self, hub: &HubId, agent: &AgentId) -> CloudFuture<Duration> {
        let (hub_id, agent) = (hub.clone(), agent.clone());
        let call = self.call(hub, move |shared, _captured, _account| async move {
            shared.leases.remaining(&hub_id, &agent)
        });
        Box::pin(async move { call.await.map_err(leases::agent_error) })
    }
}
