//! The Cloud agents ports of `nocterm-cloud-agents-model`, over the hub contract.
//!
//! Every call captures the signed-in membership before it is spawned and rechecks it when it
//! ends, as [`crate::devices`] does: a result for an account that was signed out or replaced
//! meanwhile is `Stale`. Pool keys are pinned per hub on first use (the person compares the
//! fingerprint), and a host's setup document is imported only after its attestation verified
//! under the pinned pool key.

mod catalog;
mod convert;
mod error;
mod launch;
mod personal;
mod pins;

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use nocterm_cloud_agents_model::{
    AgentHints, AgentId, CloudAgent, CloudAgentCatalog, CloudAgentError, CloudAgentLauncher,
    CloudFuture, PersonalAgentDraft, PersonalAgents, PoolChoice, PoolOffer, SaveOutcome,
    StartOutcome, StepSink,
};
use nocterm_hubs_model::HubId;
use tokio::runtime::Handle;

use crate::devices::Captured;
use crate::links::HubLinks;
use crate::pool::Pool;
use crate::task::spawn;
use error::hub_error;
use pins::PoolPins;

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

impl PersonalAgents for CloudAgentsLink {
    fn save(
        &self,
        hub: &HubId,
        existing: Option<(AgentId, u64)>,
        draft: PersonalAgentDraft,
    ) -> CloudFuture<SaveOutcome> {
        let pins = self.pins.clone();
        self.call(hub, move |captured| {
            personal::save(captured, pins, existing, draft)
        })
    }

    fn delete(&self, hub: &HubId, agent: &AgentId, version: u64) -> CloudFuture<()> {
        let agent = agent.clone();
        self.call(hub, move |captured| {
            personal::delete(captured, agent, version)
        })
    }
}
