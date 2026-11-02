//! The entity every window reads the Cloud agents from.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui::{AppContext as _, Context, EventEmitter};
use nocterm_cloud_agents_model::{
    AgentId, CloudAgent, CloudAgentError, InstanceId, ModelUsage, PersonalAgentDraft, PinnedPool,
    Placement, PoolChoice, PoolId, PoolOffer, SaveOutcome, StartOutcome, StartStep, StartedAgent,
    StepSink,
};
use nocterm_hubs_model::HubId;

use crate::catalogs::{CatalogStatus, Catalogs};
use crate::installed::Deps;
use crate::starts::{StartId, StartPhase, StartView, Starts};

const FIRST_RETRY: Duration = Duration::from_secs(2);
const MAX_RETRY: Duration = Duration::from_secs(60);

/// What changed for the views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudAgentsEvent {
    /// A catalog or a start changed.
    Changed,
    /// A start is ready: open its chat.
    Ready(StartId),
}

/// The Cloud agents of every tracked hub and the starts under way.
pub struct CloudAgents {
    deps: Deps,
    catalogs: Catalogs,
    starts: Starts,
}

impl EventEmitter<CloudAgentsEvent> for CloudAgents {}

impl CloudAgents {
    pub(crate) fn new(deps: Deps) -> Self {
        Self {
            deps,
            catalogs: Catalogs::default(),
            starts: Starts::default(),
        }
    }

    /// Starts keeping the agents of `hub` fresh. Tracking twice is harmless.
    pub fn track(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        if self.catalogs.contains(hub) {
            return;
        }
        let catalog = self.deps.catalog.clone();
        let hub_id = hub.clone();
        let watcher = cx.spawn(async move |this, cx| {
            let mut delay = FIRST_RETRY;
            loop {
                let outcome = watch_once(&catalog, &hub_id, &this, &mut delay, cx).await;
                match outcome {
                    Err(CloudAgentError::Unsupported) => {
                        this.update(cx, |this, cx| {
                            if this.catalogs.settled(&hub_id, CatalogStatus::Unsupported) {
                                cx.emit(CloudAgentsEvent::Changed);
                                cx.notify();
                            }
                        })
                        .ok();
                        return;
                    }
                    Err(error) => {
                        tracing::debug!(%error, "cloud agent catalog call failed");
                        let alive = this
                            .update(cx, |this, cx| {
                                if this.catalogs.settled(&hub_id, CatalogStatus::Failed) {
                                    cx.emit(CloudAgentsEvent::Changed);
                                    cx.notify();
                                }
                            })
                            .is_ok();
                        if !alive {
                            return;
                        }
                    }
                    Ok(()) => {}
                }
                cx.background_executor().timer(delay).await;
                delay = (delay * 2).min(MAX_RETRY);
            }
        });
        self.catalogs.insert(hub.clone(), watcher);
        cx.emit(CloudAgentsEvent::Changed);
        cx.notify();
    }

    /// Stops tracking `hub` (it was removed or signed out) and forgets its agents and starts.
    pub fn untrack(&mut self, hub: &HubId, cx: &mut Context<Self>) {
        let removed = self.catalogs.remove(hub);
        let dropped = self.starts.drop_hub(hub);
        if removed || dropped {
            cx.emit(CloudAgentsEvent::Changed);
            cx.notify();
        }
    }

    /// The agents listed for `hub`.
    pub fn agents(&self, hub: &HubId) -> &[CloudAgent] {
        self.catalogs.agents(hub)
    }

    /// How the catalog of `hub` stands; `None` when the hub is not tracked.
    pub fn status(&self, hub: &HubId) -> Option<CatalogStatus> {
        self.catalogs.status(hub)
    }

    /// Whether the Cloud tab has anything to offer for `hub`.
    pub fn is_offered(&self, hub: &HubId) -> bool {
        !matches!(self.status(hub), None | Some(CatalogStatus::Unsupported))
    }

    /// The starts of this session.
    pub fn starts(&self) -> impl Iterator<Item = StartView<'_>> {
        self.starts.iter()
    }

    /// One start.
    pub fn start_view(&self, id: StartId) -> Option<StartView<'_>> {
        self.starts.get(id)
    }

    /// Forgets a finished start.
    pub fn dismiss(&mut self, id: StartId, cx: &mut Context<Self>) {
        if self.starts.dismiss(id) {
            cx.emit(CloudAgentsEvent::Changed);
            cx.notify();
        }
    }

    /// Starts `agent` of `hub`. An agent that is already starting or ready returns its start; a
    /// failed start is replaced by a new one.
    pub fn start(&mut self, hub: &HubId, agent: &CloudAgent, cx: &mut Context<Self>) -> StartId {
        self.launch(hub, &agent.id, &agent.name, agent.placement(), cx)
    }

    /// Starts again a start that failed. `None` when the start is gone or did not fail.
    pub fn retry(&mut self, id: StartId, cx: &mut Context<Self>) -> Option<StartId> {
        let (hub, agent, name, placement) = {
            let view = self.starts.get(id)?;
            if !matches!(view.phase(), StartPhase::Failed(_)) {
                return None;
            }
            (
                view.hub().clone(),
                view.agent().clone(),
                view.name().to_owned(),
                view.placement(),
            )
        };
        Some(self.launch(&hub, &agent, &name, placement, cx))
    }

    fn launch(
        &mut self,
        hub: &HubId,
        agent: &AgentId,
        name: &str,
        placement: Placement,
        cx: &mut Context<Self>,
    ) -> StartId {
        let (id, fresh) = self.starts.begin(hub, agent, name, placement);
        if fresh {
            match placement {
                Placement::Pool => self.run_start(id, hub.clone(), agent.clone(), cx),
                Placement::OwnRunner => self.run_own_start(id, hub.clone(), agent.clone(), cx),
            }
            cx.emit(CloudAgentsEvent::Changed);
            cx.notify();
        }
        id
    }

    /// The person compared the fingerprint and trusts the pool: pins it and continues.
    pub fn trust_pool(&mut self, id: StartId, cx: &mut Context<Self>) {
        let Some(offer) = self.starts.pending_offer(id) else {
            return;
        };
        let Some((hub, agent)) = self
            .starts
            .get(id)
            .map(|view| (view.hub().clone(), view.agent().clone()))
        else {
            return;
        };
        self.starts.resume(id);
        cx.emit(CloudAgentsEvent::Changed);
        cx.notify();
        let launcher = self.deps.launcher.clone();
        cx.spawn(async move |this, cx| {
            let pinned = launcher.pin_pool(&hub, &offer).await;
            this.update(cx, |this, cx| match pinned {
                Ok(()) => this.run_start(id, hub, agent, cx),
                Err(error) => this.failed(id, error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// The person declined to trust the pool: the start ends.
    pub fn refuse_pool(&mut self, id: StartId, cx: &mut Context<Self>) {
        if self.starts.pending_offer(id).is_some() {
            self.failed(id, CloudAgentError::PoolNotTrusted, cx);
        }
    }

    /// The pools a personal agent of `hub` may be sealed to.
    pub fn pools(
        &self,
        hub: &HubId,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<Vec<PoolChoice>, CloudAgentError>> {
        let call = self.deps.catalog.pools(hub);
        cx.background_spawn(call)
    }

    /// The pools pinned on this device for `hub`.
    pub fn pinned_pools(
        &self,
        hub: &HubId,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<Vec<PinnedPool>, CloudAgentError>> {
        cx.background_spawn(self.deps.pins.pinned(hub))
    }

    /// Forgets the pin of `pool` on `hub`; the next start on that pool asks for trust again.
    pub fn forget_pool(
        &self,
        hub: &HubId,
        pool: &PoolId,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<(), CloudAgentError>> {
        cx.background_spawn(self.deps.pins.forget(hub, pool))
    }

    /// What the person's own agents consumed on `hub` in the last `days` days, per model.
    pub fn personal_usage(
        &self,
        hub: &HubId,
        days: u32,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<Vec<ModelUsage>, CloudAgentError>> {
        cx.background_spawn(self.deps.usage.personal_usage(hub, days))
    }

    /// Saves a personal agent; the catalog refreshes through the hub's change hint.
    pub fn save_personal(
        &self,
        hub: &HubId,
        existing: Option<(AgentId, u64)>,
        draft: PersonalAgentDraft,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<SaveOutcome, CloudAgentError>> {
        let call = self.deps.personal.save(hub, existing, draft);
        cx.background_spawn(call)
    }

    /// Pins a pool the person trusts while saving a personal agent.
    pub fn pin_pool(
        &self,
        hub: &HubId,
        offer: &PoolOffer,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<(), CloudAgentError>> {
        let call = self.deps.launcher.pin_pool(hub, offer);
        cx.background_spawn(call)
    }

    /// Deletes a personal agent.
    pub fn delete_personal(
        &self,
        hub: &HubId,
        agent: &AgentId,
        version: u64,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<(), CloudAgentError>> {
        let call = self.deps.personal.delete(hub, agent, version);
        cx.background_spawn(call)
    }

    fn run_start(&mut self, id: StartId, hub: HubId, agent: AgentId, cx: &mut Context<Self>) {
        let launcher = self.deps.launcher.clone();
        let (sender, mut steps) = mpsc::unbounded::<StartStep>();
        let sink: StepSink = Arc::new(move |step| {
            sender.unbounded_send(step).ok();
        });
        let mut call = launcher.start(&hub, &agent, sink).fuse();
        cx.spawn(async move |this, cx| {
            let result = loop {
                futures::select_biased! {
                    step = steps.next() => {
                        if let Some(step) = step
                            && this.update(cx, |this, cx| this.stepped(id, step, cx)).is_err()
                        {
                            return;
                        }
                    }
                    result = call => break result,
                }
            };
            while let Ok(step) = steps.try_recv() {
                this.update(cx, |this, cx| this.stepped(id, step, cx)).ok();
            }
            this.update(cx, |this, cx| match result {
                Ok(StartOutcome::Ready(started)) => {
                    if this.starts.finish(id, started) {
                        cx.emit(CloudAgentsEvent::Ready(id));
                        cx.emit(CloudAgentsEvent::Changed);
                        cx.notify();
                    }
                }
                Ok(StartOutcome::NeedsPoolTrust(offer)) => {
                    if this.starts.await_trust(id, offer) {
                        cx.emit(CloudAgentsEvent::Changed);
                        cx.notify();
                    }
                }
                Err(error) => this.failed(id, error, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Starts an agent that works only while this device is online: the device leases the
    /// credential and keeps renewing it for as long as the start is listed and the hub answers.
    fn run_own_start(&mut self, id: StartId, hub: HubId, agent: AgentId, cx: &mut Context<Self>) {
        let launcher = self.deps.own_runner.clone();
        let (sender, mut steps) = mpsc::unbounded::<StartStep>();
        let sink: StepSink = Arc::new(move |step| {
            sender.unbounded_send(step).ok();
        });
        let mut call = launcher.start(&hub, &agent, sink).fuse();
        cx.spawn(async move |this, cx| {
            let result = loop {
                futures::select_biased! {
                    step = steps.next() => {
                        if let Some(step) = step
                            && this.update(cx, |this, cx| this.stepped(id, step, cx)).is_err()
                        {
                            return;
                        }
                    }
                    result = call => break result,
                }
            };
            while let Ok(step) = steps.try_recv() {
                this.update(cx, |this, cx| this.stepped(id, step, cx)).ok();
            }
            let seat = match result {
                Ok(seat) => seat,
                Err(error) => {
                    this.update(cx, |this, cx| this.failed(id, error, cx)).ok();
                    return;
                }
            };
            let started = StartedAgent {
                agent: agent.clone(),
                instance: own_instance(&agent),
                runner: seat.runner,
            };
            let live = this
                .update(cx, |this, cx| {
                    let finished = this.starts.finish(id, started);
                    if finished {
                        cx.emit(CloudAgentsEvent::Ready(id));
                        cx.emit(CloudAgentsEvent::Changed);
                        cx.notify();
                    }
                    finished
                })
                .unwrap_or(false);
            if live {
                renew_lease(&launcher, &hub, &agent, id, seat.lease_ttl, &this, cx).await;
            }
        })
        .detach();
    }

    fn stepped(&mut self, id: StartId, step: StartStep, cx: &mut Context<Self>) {
        if self.starts.step(id, step) {
            cx.emit(CloudAgentsEvent::Changed);
            cx.notify();
        }
    }

    fn failed(&mut self, id: StartId, error: CloudAgentError, cx: &mut Context<Self>) {
        if self.starts.fail(id, error) {
            cx.emit(CloudAgentsEvent::Changed);
            cx.notify();
        }
    }

    pub(crate) fn listed(&mut self, hub: &HubId, agents: Vec<CloudAgent>, cx: &mut Context<Self>) {
        if self.catalogs.listed(hub, agents) {
            cx.emit(CloudAgentsEvent::Changed);
            cx.notify();
        }
    }
}

/// Lists, then waits for hints and lists again until the stream ends or a call fails.
async fn watch_once(
    catalog: &Arc<dyn nocterm_cloud_agents_model::CloudAgentCatalog>,
    hub: &HubId,
    this: &gpui::WeakEntity<CloudAgents>,
    delay: &mut Duration,
    cx: &mut gpui::AsyncApp,
) -> Result<(), CloudAgentError> {
    let mut hints = catalog.watch(hub).await?;
    loop {
        let agents = catalog.list(hub).await?;
        *delay = FIRST_RETRY;
        let hub_id = hub.clone();
        if this
            .update(cx, |this, cx| this.listed(&hub_id, agents, cx))
            .is_err()
        {
            return Ok(());
        }
        if hints.next().await?.is_none() {
            return Ok(());
        }
    }
}

/// The seat label of an agent on the own runner: the device holds it, no host does.
fn own_instance(agent: &AgentId) -> InstanceId {
    InstanceId::new(format!("own-{agent}"))
        .or_else(|_| InstanceId::new("own"))
        .unwrap_or_else(|_| unreachable!("a short ASCII identifier is valid"))
}

/// Renews the lease at half its term until the start is dismissed or its hub untracked, the
/// service is gone, or a renewal fails (the device went offline or the hub ended the lease).
async fn renew_lease(
    launcher: &Arc<dyn nocterm_cloud_agents_model::OwnRunnerLauncher>,
    hub: &HubId,
    agent: &AgentId,
    id: StartId,
    mut term: Duration,
    this: &gpui::WeakEntity<CloudAgents>,
    cx: &mut gpui::AsyncApp,
) {
    loop {
        cx.background_executor().timer(term / 2).await;
        let listed = this
            .read_with(cx, |this, _| this.starts.get(id).is_some())
            .unwrap_or(false);
        if !listed {
            return;
        }
        match launcher.renew(hub, agent).await {
            Ok(next) => term = next,
            Err(error) => {
                tracing::debug!(%error, "own runner lease renewal ended");
                return;
            }
        }
    }
}
