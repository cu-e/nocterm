//! The runs of starts under way or finished, as pure state transitions.

use std::sync::Arc;

use nocterm_cloud_agents_model::{AgentId, CloudAgentError, PoolOffer, StartStep, StartedAgent};
use nocterm_hubs_model::HubId;

/// Identifies one start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StartId(u64);

/// Where a start is.
#[derive(Clone, Debug)]
pub enum StartPhase {
    /// Working through a step.
    Step(StartStep),
    /// Waiting for the person to trust the pool whose fingerprint is shown.
    AwaitingTrust(PoolOffer),
    /// Done: the chat can open.
    Ready(StartedAgent),
    /// Ended without a seat; the reason is typed.
    Failed(Arc<CloudAgentError>),
}

struct Run {
    id: StartId,
    hub: HubId,
    agent: AgentId,
    name: String,
    phase: StartPhase,
}

/// A read-only view of one start.
#[derive(Clone, Copy)]
pub struct StartView<'a>(&'a Run);

impl StartView<'_> {
    /// The start's id.
    pub fn id(&self) -> StartId {
        self.0.id
    }
    /// The hub.
    pub fn hub(&self) -> &HubId {
        &self.0.hub
    }
    /// The agent.
    pub fn agent(&self) -> &AgentId {
        &self.0.agent
    }
    /// The agent's name as the person saw it.
    pub fn name(&self) -> &str {
        &self.0.name
    }
    /// Where the start is.
    pub fn phase(&self) -> &StartPhase {
        &self.0.phase
    }
    /// Whether the start has ended, one way or the other.
    pub fn is_finished(&self) -> bool {
        matches!(self.0.phase, StartPhase::Ready(_) | StartPhase::Failed(_))
    }
}

/// The starts of this session. Finished runs are kept until dismissed so a view that opens
/// later still shows how they ended.
#[derive(Default)]
pub(crate) struct Starts {
    runs: Vec<Run>,
    next: u64,
}

impl Starts {
    /// A new run in its first step. A running start of the same agent is returned instead of
    /// beginning a second one.
    pub(crate) fn begin(&mut self, hub: &HubId, agent: &AgentId, name: &str) -> (StartId, bool) {
        if let Some(run) = self
            .runs
            .iter()
            .find(|run| &run.hub == hub && &run.agent == agent && !finished(&run.phase))
        {
            return (run.id, false);
        }
        self.next += 1;
        let id = StartId(self.next);
        self.runs.push(Run {
            id,
            hub: hub.clone(),
            agent: agent.clone(),
            name: name.to_owned(),
            phase: StartPhase::Step(StartStep::ReservingSeat),
        });
        (id, true)
    }

    pub(crate) fn get(&self, id: StartId) -> Option<StartView<'_>> {
        self.runs.iter().find(|run| run.id == id).map(StartView)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = StartView<'_>> {
        self.runs.iter().map(StartView)
    }

    /// Records a step reported by the launcher. A finished run ignores late steps.
    pub(crate) fn step(&mut self, id: StartId, step: StartStep) -> bool {
        let Some(run) = self.live(id) else {
            return false;
        };
        run.phase = match step {
            StartStep::TrustingPool(offer) => StartPhase::AwaitingTrust(offer),
            StartStep::Ready => return false,
            other => StartPhase::Step(other),
        };
        true
    }

    pub(crate) fn await_trust(&mut self, id: StartId, offer: PoolOffer) -> bool {
        self.set_live(id, StartPhase::AwaitingTrust(offer))
    }

    /// The offer a person is asked about, if the run waits for one.
    pub(crate) fn pending_offer(&self, id: StartId) -> Option<PoolOffer> {
        match &self.runs.iter().find(|run| run.id == id)?.phase {
            StartPhase::AwaitingTrust(offer) => Some(offer.clone()),
            _ => None,
        }
    }

    pub(crate) fn resume(&mut self, id: StartId) -> bool {
        self.set_live(id, StartPhase::Step(StartStep::ReservingSeat))
    }

    pub(crate) fn finish(&mut self, id: StartId, started: StartedAgent) -> bool {
        self.set_live(id, StartPhase::Ready(started))
    }

    pub(crate) fn fail(&mut self, id: StartId, error: CloudAgentError) -> bool {
        self.set_live(id, StartPhase::Failed(Arc::new(error)))
    }

    /// Forgets a finished run.
    pub(crate) fn dismiss(&mut self, id: StartId) -> bool {
        let before = self.runs.len();
        self.runs
            .retain(|run| run.id != id || !finished(&run.phase));
        self.runs.len() != before
    }

    /// Forgets every run of a hub (it signed out).
    pub(crate) fn drop_hub(&mut self, hub: &HubId) -> bool {
        let before = self.runs.len();
        self.runs.retain(|run| &run.hub != hub);
        self.runs.len() != before
    }

    fn live(&mut self, id: StartId) -> Option<&mut Run> {
        self.runs
            .iter_mut()
            .find(|run| run.id == id && !finished(&run.phase))
    }

    fn set_live(&mut self, id: StartId, phase: StartPhase) -> bool {
        match self.live(id) {
            Some(run) => {
                run.phase = phase;
                true
            }
            None => false,
        }
    }
}

fn finished(phase: &StartPhase) -> bool {
    matches!(phase, StartPhase::Ready(_) | StartPhase::Failed(_))
}
