//! Starting a chat with an agent: the steps a person sees and what the hub can refuse.

use std::fmt;

use crate::ids::{AgentId, InstanceId, PoolId};

/// Why the hub reserved no seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No host of the pool has free CPU, memory, disk or slots.
    HostCapacity,
    /// The organization's or the server's quota is used up.
    ScopeQuota,
    /// The person's own share of the quota is used up.
    MemberQuota,
    /// The spend budget of the period is used up.
    Budget,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::HostCapacity => "no host of the pool has free capacity",
            Self::ScopeQuota => "the quota of the organization or server is used up",
            Self::MemberQuota => "your share of the quota is used up",
            Self::Budget => "the spend budget is used up",
        })
    }
}

/// Why a host's setup document is not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupProblem {
    /// The instance carries no setup document or attestation yet.
    Missing,
    /// The attestation does not verify under the pinned pool keys, or names another instance.
    NotAttested,
    /// The document differs from the one the host attested.
    Substituted,
    /// The document is not a runner setup this device can import.
    Unreadable,
}

impl fmt::Display for SetupProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "the host sent no setup document",
            Self::NotAttested => "the host's signature does not verify under the pinned pool key",
            Self::Substituted => "the setup document is not the one the host signed",
            Self::Unreadable => "the setup document cannot be read",
        })
    }
}

/// The steps of a start in the order a person sees them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartStep {
    /// The hub reserves disk and a slot and the host prepares the runner.
    ReservingSeat,
    /// The pool's key is unknown here: waiting for the person to compare the fingerprint.
    TrustingPool(PoolOffer),
    /// The device imports the runner and gives it its grant.
    Registering,
    /// The agent is ready; the chat opens.
    Ready,
}

impl StartStep {
    /// The line a progress view shows.
    pub fn text(&self) -> &'static str {
        match self {
            Self::ReservingSeat => "Reserving space…",
            Self::TrustingPool(_) => "Pool trust",
            Self::Registering => "Registering…",
            Self::Ready => "Ready",
        }
    }
}

/// A pool whose keys are not pinned yet, with the fingerprint to compare out of band.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolOffer {
    /// The pool.
    pub pool: PoolId,
    /// The pool's name.
    pub name: String,
    /// The fingerprint of both pool keys, to compare with the one the pool's owner publishes.
    pub fingerprint: String,
}

/// A seat that is registered and ready for a chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartedAgent {
    /// The agent.
    pub agent: AgentId,
    /// The seat.
    pub instance: InstanceId,
    /// The runner's identifier on the hub (16 bytes), once the host created it.
    pub runner: Vec<u8>,
}
