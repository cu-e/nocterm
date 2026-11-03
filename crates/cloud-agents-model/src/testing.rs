//! In-memory implementations of the Cloud agents ports for consumers' tests.
//!
//! Enable `test-support` in a dev-dependency. A call completes when it is polled. Clones share
//! one state, so a test keeps a handle while the code under test holds another.

mod chats;
pub use chats::{ChatFailure, FakeCloudChatLauncher};

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::channel::{mpsc, oneshot};
use futures::future::{BoxFuture, Shared};
use nocterm_hubs_model::HubId;

use crate::agent::CloudAgent;
use crate::error::CloudAgentError;
use crate::ids::{AgentId, InstanceId, PoolId};
use crate::personal::PersonalAgentDraft;
use crate::ports::{
    AgentHints, AgentSeats, AgentUsage, CloudAgentCatalog, CloudAgentLauncher, CloudFuture,
    OwnRunnerLauncher, OwnRunnerSeat, PersonalAgents, PinnedPool, PoolChoice, PoolPinStore,
    SaveOutcome, StartOutcome, StepSink,
};
use crate::start::{PoolOffer, StartStep, StartedAgent};
use crate::usage::{ModelUsage, UsageSample};

/// How the next start of an agent ends.
#[derive(Clone, Debug)]
pub enum StartScript {
    /// Every step, then ready.
    Succeed,
    /// The hub fails the call.
    Fail(FakeFailure),
}

/// A failure a test scripts; converted to [`CloudAgentError`] when the call runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeFailure {
    /// The hub offers no managed agents.
    Unsupported,
    /// No host has free capacity.
    HostCapacity,
    /// The pool's keys changed.
    PoolKeyChanged,
    /// The account changed meanwhile.
    Stale,
    /// The host could not create the agent's runner.
    ProvisioningFailed,
}

impl FakeFailure {
    fn error(self) -> CloudAgentError {
        match self {
            Self::Unsupported => CloudAgentError::Unsupported,
            Self::HostCapacity => CloudAgentError::Refused(crate::start::Refusal::HostCapacity),
            Self::PoolKeyChanged => CloudAgentError::PoolKeyChanged {
                pinned: "AAAA".into(),
                offered: "BBBB".into(),
            },
            Self::Stale => CloudAgentError::Stale,
            Self::ProvisioningFailed => CloudAgentError::ProvisioningFailed {
                instance: crate::InstanceId::new("inst-1").expect("instance id"),
                reason: None,
            },
        }
    }
}

/// A personal agent the fake stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedAgent {
    /// The id the fake gave it.
    pub id: AgentId,
    /// Its version.
    pub version: u64,
    /// The name the form had.
    pub name: String,
    /// Whether it works while the person is offline.
    pub work_offline: bool,
    /// The instructions the form had.
    pub instructions: String,
}

/// What the own runner was asked.
#[derive(Default)]
struct OwnRunnerLog {
    starts: Vec<AgentId>,
    renewals: Vec<AgentId>,
    renewals_fail_after: Option<usize>,
}

/// The seats the person stopped or removed.
#[derive(Default)]
struct SeatLog {
    stopped: Vec<AgentId>,
    removed: Vec<AgentId>,
    /// The agents the person has a seat of.
    seated: Vec<AgentId>,
    /// Stop and remove wait for this to resolve while it is set.
    gate: Option<Shared<oneshot::Receiver<()>>>,
}

#[derive(Default)]
struct State {
    agents: BTreeMap<HubId, Vec<CloudAgent>>,
    failing: BTreeMap<HubId, FakeFailure>,
    watchers: Vec<(HubId, mpsc::UnboundedSender<u64>)>,
    pools: BTreeMap<HubId, Vec<PoolChoice>>,
    pinned: BTreeMap<(HubId, PoolId), String>,
    offers: BTreeMap<AgentId, PoolOffer>,
    scripts: BTreeMap<AgentId, VecDeque<StartScript>>,
    starts: Vec<AgentId>,
    usage: BTreeMap<HubId, Vec<UsageSample>>,
    own: OwnRunnerLog,
    saved: Vec<SavedAgent>,
    web_urls: BTreeMap<HubId, String>,
    seats: SeatLog,
    next: u64,
    revision: u64,
}

/// The catalog, launcher and personal agents of fake hubs.
#[derive(Clone, Default)]
pub struct FakeCloudAgents {
    state: Arc<Mutex<State>>,
}

impl FakeCloudAgents {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The hub lists exactly these agents from now on, and watchers get a hint.
    pub fn set_agents(&self, hub: &HubId, agents: Vec<CloudAgent>) {
        let mut state = self.state();
        state.agents.insert(hub.clone(), agents);
        state.revision += 1;
        let revision = state.revision;
        state
            .watchers
            .retain(|(watched, sender)| watched != hub || sender.unbounded_send(revision).is_ok());
    }

    /// Every call about `hub` fails like this until [`Self::heal`].
    pub fn fail(&self, hub: &HubId, failure: FakeFailure) {
        self.state().failing.insert(hub.clone(), failure);
    }

    /// Calls about `hub` work again.
    pub fn heal(&self, hub: &HubId) {
        self.state().failing.remove(hub);
    }

    /// The pools a personal agent may use on `hub`.
    pub fn set_pools(&self, hub: &HubId, pools: Vec<PoolChoice>) {
        self.state().pools.insert(hub.clone(), pools);
    }

    /// Starting `agent` needs the person to trust this pool, until it is pinned.
    pub fn require_trust(&self, agent: &AgentId, offer: PoolOffer) {
        self.state().offers.insert(agent.clone(), offer);
    }

    /// The next start of `agent` ends as scripted (scripts are used in order).
    pub fn script_start(&self, agent: &AgentId, script: StartScript) {
        self.state()
            .scripts
            .entry(agent.clone())
            .or_default()
            .push_back(script);
    }

    /// Pins `pool` on `hub` as if the person had trusted it earlier.
    pub fn seed_pin(&self, hub: &HubId, pool: &PoolId, fingerprint: &str) {
        self.state()
            .pinned
            .insert((hub.clone(), pool.clone()), fingerprint.to_owned());
    }

    /// Whether the pool's keys were pinned through the launcher.
    pub fn is_pinned(&self, hub: &HubId, pool: &PoolId) -> bool {
        self.state()
            .pinned
            .contains_key(&(hub.clone(), pool.clone()))
    }

    /// Agents started so far, in order, including repeated attempts.
    pub fn started(&self) -> Vec<AgentId> {
        self.state().starts.clone()
    }

    /// The hub reports these usage samples for the person's own agents.
    pub fn set_usage(&self, hub: &HubId, samples: Vec<UsageSample>) {
        self.state().usage.insert(hub.clone(), samples);
    }

    /// Agents started on the own runner so far, in order.
    pub fn own_runner_started(&self) -> Vec<AgentId> {
        self.state().own.starts.clone()
    }

    /// Lease renewals so far, in order.
    pub fn renewed(&self) -> Vec<AgentId> {
        self.state().own.renewals.clone()
    }

    /// Renewals fail after this many have succeeded (the device went offline).
    pub fn fail_renewals_after(&self, successes: usize) {
        self.state().own.renewals_fail_after = Some(successes);
    }

    /// The hub states this address for its web panel.
    pub fn set_web_url(&self, hub: &HubId, url: &str) {
        self.state().web_urls.insert(hub.clone(), url.to_owned());
    }

    /// Seats stopped so far, in order.
    pub fn stopped(&self) -> Vec<AgentId> {
        self.state().seats.stopped.clone()
    }

    /// Seats removed so far, in order.
    pub fn removed(&self) -> Vec<AgentId> {
        self.state().seats.removed.clone()
    }

    /// The person has a seat of `agent` from an earlier session, without starting it here.
    pub fn set_seated(&self, agent: &AgentId) {
        let mut state = self.state();
        if !state.seats.seated.contains(agent) {
            state.seats.seated.push(agent.clone());
        }
    }

    /// The seat of `agent` was removed outside this session (the web panel, another device).
    pub fn set_unseated(&self, agent: &AgentId) {
        self.state().seats.seated.retain(|seated| seated != agent);
    }

    /// Stop and remove do not answer until the returned sender is used or dropped.
    pub fn hold_seat_calls(&self) -> oneshot::Sender<()> {
        let (sender, receiver) = oneshot::channel();
        self.state().seats.gate = Some(receiver.shared());
        sender
    }

    /// Personal agents saved so far.
    pub fn saved(&self) -> Vec<SavedAgent> {
        self.state().saved.clone()
    }

    fn check(&self, hub: &HubId) -> Result<(), CloudAgentError> {
        match self.state().failing.get(hub) {
            Some(failure) => Err(failure.error()),
            None => Ok(()),
        }
    }
}

struct FakeHints(mpsc::UnboundedReceiver<u64>);

impl AgentHints for FakeHints {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<u64>, CloudAgentError>> {
        async move { Ok(self.0.next().await) }.boxed()
    }
}

impl CloudAgentCatalog for FakeCloudAgents {
    fn list(&self, hub: &HubId) -> CloudFuture<Vec<CloudAgent>> {
        let result = self
            .check(hub)
            .map(|()| self.state().agents.get(hub).cloned().unwrap_or_default());
        async move { result }.boxed()
    }

    fn watch(&self, hub: &HubId) -> CloudFuture<Box<dyn AgentHints>> {
        let result = self.check(hub).map(|()| {
            let (sender, receiver) = mpsc::unbounded();
            self.state().watchers.push((hub.clone(), sender));
            Box::new(FakeHints(receiver)) as Box<dyn AgentHints>
        });
        async move { result }.boxed()
    }

    fn pools(&self, hub: &HubId) -> CloudFuture<Vec<PoolChoice>> {
        let result = self
            .check(hub)
            .map(|()| self.state().pools.get(hub).cloned().unwrap_or_default());
        async move { result }.boxed()
    }

    fn web_url(&self, hub: &HubId) -> CloudFuture<Option<String>> {
        let result = self
            .check(hub)
            .map(|()| self.state().web_urls.get(hub).cloned());
        async move { result }.boxed()
    }
}

impl AgentSeats for FakeCloudAgents {
    fn seated(&self, hub: &HubId) -> CloudFuture<Vec<AgentId>> {
        let result = self.check(hub).map(|()| self.state().seats.seated.clone());
        async move { result }.boxed()
    }

    fn stop(&self, hub: &HubId, agent: &AgentId) -> CloudFuture<()> {
        let gate = self.state().seats.gate.clone();
        let (this, hub, agent) = (self.clone(), hub.clone(), agent.clone());
        async move {
            if let Some(gate) = gate {
                gate.await.ok();
            }
            this.check(&hub)
                .map(|()| this.state().seats.stopped.push(agent))
        }
        .boxed()
    }

    fn remove(&self, hub: &HubId, agent: &AgentId) -> CloudFuture<()> {
        let gate = self.state().seats.gate.clone();
        let (this, hub, agent) = (self.clone(), hub.clone(), agent.clone());
        async move {
            if let Some(gate) = gate {
                gate.await.ok();
            }
            this.check(&hub).map(|()| {
                let mut state = this.state();
                state.seats.seated.retain(|seated| seated != &agent);
                state.seats.removed.push(agent);
            })
        }
        .boxed()
    }
}

impl CloudAgentLauncher for FakeCloudAgents {
    fn start(&self, hub: &HubId, agent: &AgentId, steps: StepSink) -> CloudFuture<StartOutcome> {
        let result = (|| {
            self.check(hub)?;
            let mut state = self.state();
            state.starts.push(agent.clone());
            let script = state
                .scripts
                .get_mut(agent)
                .and_then(VecDeque::pop_front)
                .unwrap_or(StartScript::Succeed);
            steps(StartStep::ReservingSeat);
            if let StartScript::Fail(failure) = script {
                return Err(failure.error());
            }
            if let Some(offer) = state.offers.get(agent)
                && !state
                    .pinned
                    .contains_key(&(hub.clone(), offer.pool.clone()))
            {
                let offer = offer.clone();
                steps(StartStep::TrustingPool(offer.clone()));
                return Ok(StartOutcome::NeedsPoolTrust(offer));
            }
            steps(StartStep::Registering);
            steps(StartStep::Ready);
            if !state.seats.seated.contains(agent) {
                state.seats.seated.push(agent.clone());
            }
            Ok(StartOutcome::Ready(StartedAgent {
                agent: agent.clone(),
                instance: InstanceId::new(format!("seat-{agent}"))
                    .unwrap_or_else(|_| InstanceId::new("seat").unwrap_or_else(|_| unreachable!())),
                runner: vec![7; 16],
            }))
        })();
        async move { result }.boxed()
    }

    fn pin_pool(&self, hub: &HubId, offer: &PoolOffer) -> CloudFuture<()> {
        let result = self.check(hub).map(|()| {
            self.state()
                .pinned
                .insert((hub.clone(), offer.pool.clone()), offer.fingerprint.clone());
        });
        async move { result }.boxed()
    }
}

impl PoolPinStore for FakeCloudAgents {
    fn pinned(&self, hub: &HubId) -> CloudFuture<Vec<PinnedPool>> {
        let result = self.check(hub).map(|()| {
            self.state()
                .pinned
                .iter()
                .filter(|((pinned_on, _), _)| pinned_on == hub)
                .map(|((_, pool), fingerprint)| PinnedPool {
                    pool: pool.clone(),
                    fingerprint: fingerprint.clone(),
                })
                .collect()
        });
        async move { result }.boxed()
    }

    fn forget(&self, hub: &HubId, pool: &PoolId) -> CloudFuture<()> {
        let result = self.check(hub).map(|()| {
            self.state().pinned.remove(&(hub.clone(), pool.clone()));
        });
        async move { result }.boxed()
    }
}

impl PersonalAgents for FakeCloudAgents {
    fn save(
        &self,
        hub: &HubId,
        existing: Option<(AgentId, u64)>,
        draft: PersonalAgentDraft,
    ) -> CloudFuture<SaveOutcome> {
        let result = (|| {
            self.check(hub)?;
            let mut state = self.state();
            let (id, version) = match existing {
                Some((id, version)) => {
                    state.saved.retain(|saved| saved.id != id);
                    (id, version + 1)
                }
                None => {
                    state.next += 1;
                    let id = AgentId::new(format!("personal-{}", state.next))
                        .unwrap_or_else(|_| unreachable!());
                    (id, 1)
                }
            };
            state.saved.push(SavedAgent {
                id: id.clone(),
                version,
                name: draft.name().to_owned(),
                work_offline: draft.work_offline(),
                instructions: draft.instructions().to_owned(),
            });
            Ok(SaveOutcome::Saved(id))
        })();
        async move { result }.boxed()
    }

    fn delete(&self, hub: &HubId, agent: &AgentId, _version: u64) -> CloudFuture<()> {
        let result = self.check(hub).map(|()| {
            self.state().saved.retain(|saved| &saved.id != agent);
        });
        async move { result }.boxed()
    }
}

/// The lease term the fake grants.
pub const FAKE_LEASE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

impl OwnRunnerLauncher for FakeCloudAgents {
    fn start(&self, hub: &HubId, agent: &AgentId, steps: StepSink) -> CloudFuture<OwnRunnerSeat> {
        let result = self.check(hub).map(|()| {
            self.state().own.starts.push(agent.clone());
            steps(StartStep::Registering);
            steps(StartStep::Ready);
            OwnRunnerSeat {
                runner: vec![9; 16],
                lease_ttl: FAKE_LEASE_TTL,
            }
        });
        async move { result }.boxed()
    }

    fn renew(&self, hub: &HubId, agent: &AgentId) -> CloudFuture<std::time::Duration> {
        let result = self.check(hub).and_then(|()| {
            let mut state = self.state();
            if state
                .own
                .renewals_fail_after
                .is_some_and(|limit| state.own.renewals.len() >= limit)
            {
                return Err(CloudAgentError::Stale);
            }
            state.own.renewals.push(agent.clone());
            Ok(FAKE_LEASE_TTL)
        });
        async move { result }.boxed()
    }
}

impl AgentUsage for FakeCloudAgents {
    fn personal_usage(&self, hub: &HubId, _days: u32) -> CloudFuture<Vec<ModelUsage>> {
        let result = self.check(hub).map(|()| {
            ModelUsage::by_model(self.state().usage.get(hub).cloned().unwrap_or_default())
        });
        async move { result }.boxed()
    }
}
