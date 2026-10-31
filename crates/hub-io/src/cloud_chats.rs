//! The Cloud chat port of `nocterm-cloud-agents-model`, over the hub contract and the account's
//! Cloud owners.
//!
//! Creating a chat mints its identifier and first key epoch on this device, stores the wrapped
//! key and the sealed grant in the account, and publishes the grant to the runner, which claims
//! the chat; the call returns the verified source once that claim is visible. Listing walks the
//! authenticated catalogs of the registered runners. Every call captures the signed-in
//! membership before it is spawned and rechecks it when it ends, as [`crate::agents`] does
//! ([ADR 0122](../../../docs/adr/0122-cloud-chat-creation-protocol.md)).

mod budget;
mod create;
mod directory;
mod error;
mod managed;
mod open;
mod own_runner;
mod session;
mod setup;
mod timer;
mod wait;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use nocterm_cloud_agents_model::{
    AgentId, ChatFuture, ChatHistoryEntry, ChatHost, ChatRunner, CloudChatError, CloudChatLauncher,
    CloudFuture, OpenedChat, OwnRunnerLauncher, OwnRunnerSeat, StepSink,
};
use nocterm_cloud_model::tool::ToolTimer;
use nocterm_hubs_model::HubId;
use tokio::runtime::Handle;

use crate::cloud::{ConversationStorage, ConversationWorkers};
use crate::devices::Captured;
use crate::hub_account::HubAccount;
use crate::links::HubLinks;
use crate::pool::Pool;
use crate::task::spawn;
use budget::ChatBudget;
use setup::Collaborators;

/// The installation-wide owners of conversation sources, injected by the application.
pub struct ChatServices {
    /// Bounded workers shared by every source.
    pub workers: Arc<ConversationWorkers>,
    /// The private encrypted files of the sources.
    pub storage: Arc<ConversationStorage>,
    /// The clock and delays of waits and sources.
    pub timer: Arc<dyn ToolTimer>,
}

/// Most chat namespaces that are open at once.
const MAX_OPEN_CHATS: usize = 32;
/// Most blocking operations of all sources at once.
const MAX_WORKERS: usize = 8;

impl ChatServices {
    /// The services of an application over `links`: workers and a clock on the adapter's
    /// runtime, and encrypted chat files under `root`.
    pub fn native(links: &HubLinks, root: std::path::PathBuf) -> Result<Self, CloudChatError> {
        let (handle, _) = links.agent_parts();
        let resident = ConversationStorage::resident_bytes(MAX_OPEN_CHATS)
            .map_err(|_| CloudChatError::Capacity)?;
        let lease = ChatBudget::new()?.resident(resident)?;
        Ok(Self {
            workers: ConversationWorkers::new(handle.clone(), MAX_WORKERS)
                .map_err(|_| CloudChatError::Capacity)?,
            storage: ConversationStorage::new(root, MAX_OPEN_CHATS, lease)
                .map_err(|_| CloudChatError::Capacity)?,
            timer: Arc::new(timer::NativeTimer::new(handle)),
        })
    }
}

/// What one operation shares with the others of the installation.
struct Shared {
    handle: Handle,
    budget: ChatBudget,
    collaborators: Collaborators,
    leases: own_runner::Leases,
}

impl Shared {
    fn leases(&self) -> &own_runner::Leases {
        &self.leases
    }

    fn handle(&self) -> &Handle {
        &self.handle
    }

    fn budget(&self) -> &ChatBudget {
        &self.budget
    }

    fn collaborators(&self) -> &Collaborators {
        &self.collaborators
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
}

/// Reaches the Cloud chats of the hubs this device is signed in to.
pub struct CloudChatsLink {
    links: Arc<HubLinks>,
    pool: Arc<Pool>,
    shared: Arc<Shared>,
}

impl CloudChatsLink {
    /// Fails with `Capacity` when the finite pools cannot be sized.
    pub fn new(links: Arc<HubLinks>, services: ChatServices) -> Result<Self, CloudChatError> {
        let (handle, pool) = links.agent_parts();
        let shared = Arc::new(Shared {
            handle: handle.clone(),
            budget: ChatBudget::new()?,
            collaborators: Collaborators {
                workers: services.workers,
                storage: services.storage,
                timer: services.timer,
            },
            leases: own_runner::Leases::default(),
        });
        Ok(Self {
            links,
            pool,
            shared,
        })
    }

    fn call<T, F, Fut>(&self, hub: &HubId, work: F) -> ChatFuture<T>
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
                let work = work(self.shared.clone(), captured.clone(), account);
                self.shared.run(&captured, work)
            }
            Err(failure) => Box::pin(async move { Err(failure) }),
        }
    }
}

impl CloudChatLauncher for CloudChatsLink {
    fn runners(&self, hub: &HubId) -> ChatFuture<Vec<ChatRunner>> {
        self.call(hub, |shared, captured, account| async move {
            let opened =
                session::OpenedAccount::open(&account, shared.handle(), shared.budget()).await?;
            let seats = managed::seats(&captured).await?;
            Ok(opened
                .runners()?
                .iter()
                .filter(|row| !seats.contains_key(row.binding().runner().as_bytes()))
                .map(|row| ChatRunner {
                    id: *row.binding().runner().as_bytes(),
                    label: row.label().to_owned(),
                })
                .collect())
        })
    }

    fn create(&self, hub: &HubId, host: ChatHost) -> ChatFuture<OpenedChat> {
        let hub_id = hub.clone();
        self.call(hub, move |shared, captured, account| {
            create::create(shared, captured, account, hub_id, host)
        })
    }

    fn history(&self, hub: &HubId) -> ChatFuture<Vec<Arc<dyn ChatHistoryEntry>>> {
        self.call(hub, directory::list)
    }
}

impl OwnRunnerLauncher for CloudChatsLink {
    fn start(
        &self,
        hub: &HubId,
        agent: &AgentId,
        _steps: StepSink,
    ) -> CloudFuture<OwnRunnerSeat> {
        let (hub_id, agent) = (hub.clone(), agent.clone());
        let call = self.call(hub, move |shared, captured, account| async move {
            own_runner::start(&shared, &captured, &account, &hub_id, &agent).await
        });
        Box::pin(async move { call.await.map_err(own_runner::agent_error) })
    }

    fn renew(&self, hub: &HubId, agent: &AgentId) -> CloudFuture<Duration> {
        let (hub_id, agent) = (hub.clone(), agent.clone());
        let call = self.call(hub, move |shared, _captured, account| async move {
            own_runner::renew(&shared, &account, &hub_id, &agent).await
        });
        Box::pin(async move { call.await.map_err(own_runner::agent_error) })
    }
}
