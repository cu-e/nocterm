//! The Cloud agents ports of `nocterm-cloud-agents-model`, over the hub contract.
//!
//! Every call captures the signed-in membership before it is spawned and rechecks it when it
//! ends, as [`crate::devices`] does: a result for an account that was signed out or replaced
//! meanwhile is `Stale`. Pool keys are pinned per hub on first use (the person compares the
//! fingerprint), and a host's setup document is imported only after its attestation verified
//! under the pinned pool key.

mod catalog;
mod convert;
pub(crate) mod error;
mod launch;
pub(crate) mod own_keys;
mod personal;
mod pins;
mod provider;
mod seats;
mod usage;

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use nocterm_cloud_agents_model::{
    AgentHints, AgentId, AgentSeats, AgentUsage, CloudAgent, CloudAgentCatalog, CloudAgentError,
    CloudAgentLauncher, CloudFuture, ModelUsage, PersonalAgentDraft, PersonalAgents, PinnedPool,
    PoolChoice, PoolId, PoolOffer, PoolPinStore, SaveOutcome, StartOutcome, StepSink,
};
use nocterm_hubs_model::HubId;
use tokio::runtime::Handle;

use crate::devices::Captured;
use crate::links::HubLinks;
use crate::pool::Pool;
use crate::task::spawn;
pub(crate) use catalog::list as list_agents;
pub(crate) use error::agent_error;
use error::hub_error;
pub(crate) use launch::is_seat_label;
use pins::PoolPins;
pub use provider::parse_provider_kind;

/// Reaches the managed agents of the hubs this device is signed in to.
pub struct CloudAgentsLink {
    links: Arc<HubLinks>,
    handle: Handle,
    pool: Arc<Pool>,
    pins: Arc<PoolPins>,
}

impl CloudAgentsLink {
    /// `pins_dir` is the private directory that keeps the pool keys pinned per hub.
    pub fn new(links: Arc<HubLinks>, pins_dir: PathBuf) -> Self {
        let (handle, pool) = links.agent_parts();
        Self {
            links,
            handle,
            pool,
            pins: Arc::new(PoolPins::new(pins_dir)),
        }
    }

    /// Runs `work` on the adapter's threads with the membership captured now, and checks it
    /// again before the result is returned.
    fn call<T, F, Fut>(&self, hub: &HubId, work: F) -> CloudFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Captured>) -> Fut,
        Fut: Future<Output = Result<T, CloudAgentError>> + Send + 'static,
    {
        let captured = match Captured::new(Arc::clone(&self.pool), hub) {
            Ok(captured) => Arc::new(captured),
            Err(error) => return Box::pin(async move { Err(hub_error(error)) }),
        };
        let observed = captured.observed.clone();
        let task = spawn(&self.handle, work(captured));
        Box::pin(async move {
            let result = task.await.unwrap_or(Err(CloudAgentError::Stale));
            observed.check().map_err(hub_error)?;
            result
        })
    }
}

impl CloudAgentCatalog for CloudAgentsLink {
    fn list(&self, hub: &HubId) -> CloudFuture<Vec<CloudAgent>> {
        self.call(hub, catalog::list)
    }

    fn watch(&self, hub: &HubId) -> CloudFuture<Box<dyn AgentHints>> {
        let handle = self.handle.clone();
        self.call(hub, move |captured| catalog::watch(captured, handle))
    }

    fn pools(&self, hub: &HubId) -> CloudFuture<Vec<PoolChoice>> {
        self.call(hub, catalog::pools)
    }

    fn web_url(&self, hub: &HubId) -> CloudFuture<Option<String>> {
        self.call(hub, |captured| async move {
            Ok(captured.client.web_url().map(str::to_owned))
        })
    }
}

impl AgentSeats for CloudAgentsLink {
    fn seated(&self, hub: &HubId) -> CloudFuture<Vec<AgentId>> {
        self.call(hub, seats::seated)
    }

    fn stop(&self, hub: &HubId, agent: &AgentId) -> CloudFuture<()> {
        let agent = agent.clone();
        self.call(hub, move |captured| seats::stop(captured, agent))
    }

    fn remove(&self, hub: &HubId, agent: &AgentId) -> CloudFuture<()> {
        let agent = agent.clone();
        self.call(hub, move |captured| seats::remove(captured, agent))
    }
}

impl CloudAgentLauncher for CloudAgentsLink {
    fn start(&self, hub: &HubId, agent: &AgentId, steps: StepSink) -> CloudFuture<StartOutcome> {
        let (links, pins, handle) = (self.links.clone(), self.pins.clone(), self.handle.clone());
        let agent = agent.clone();
        self.call(hub, move |captured| {
            launch::start(captured, links, pins, handle, agent, steps)
        })
    }

    fn pin_pool(&self, hub: &HubId, offer: &PoolOffer) -> CloudFuture<()> {
        let (pins, offer) = (self.pins.clone(), offer.clone());
        self.call(hub, move |captured| launch::pin(captured, pins, offer))
    }
}

impl AgentUsage for CloudAgentsLink {
    fn personal_usage(&self, hub: &HubId, days: u32) -> CloudFuture<Vec<ModelUsage>> {
        self.call(hub, move |captured| usage::personal(captured, days))
    }
}

impl PoolPinStore for CloudAgentsLink {
    fn pinned(&self, hub: &HubId) -> CloudFuture<Vec<PinnedPool>> {
        let pins = self.pins.clone();
        let hub = hub.clone();
        let task = spawn(&self.handle, async move {
            pins.list(&hub)?
                .into_iter()
                .map(|(pool, keys)| {
                    Ok(PinnedPool {
                        pool: PoolId::new(pool).map_err(|_| CloudAgentError::Stale)?,
                        fingerprint: keys.fingerprint(),
                    })
                })
                .collect()
        });
        Box::pin(async move { task.await.unwrap_or(Err(CloudAgentError::Stale)) })
    }

    fn forget(&self, hub: &HubId, pool: &PoolId) -> CloudFuture<()> {
        let (pins, hub, pool) = (self.pins.clone(), hub.clone(), pool.clone());
        let task = spawn(
            &self.handle,
            async move { pins.forget(&hub, pool.as_str()) },
        );
        Box::pin(async move { task.await.unwrap_or(Err(CloudAgentError::Stale)) })
    }
}

impl PersonalAgents for CloudAgentsLink {
    fn save(
        &self,
        hub: &HubId,
        existing: Option<(AgentId, u64)>,
        draft: PersonalAgentDraft,
    ) -> CloudFuture<SaveOutcome> {
        let (pins, links, hub_id) = (self.pins.clone(), self.links.clone(), hub.clone());
        self.call(hub, move |captured| {
            let work = links
                .open_account(&hub_id)
                .and_then(|account| account.work());
            personal::save(captured, pins, work, existing, draft)
        })
    }

    fn delete(&self, hub: &HubId, agent: &AgentId, version: u64) -> CloudFuture<()> {
        let (agent, links, hub_id) = (agent.clone(), self.links.clone(), hub.clone());
        self.call(hub, move |captured| {
            let work = links
                .open_account(&hub_id)
                .and_then(|account| account.work());
            personal::delete(captured, work, agent, version)
        })
    }
}
