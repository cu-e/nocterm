//! The ports through which the desktop reaches the managed agents of a hub.
//!
//! Each port is named for what its consumer needs. A hub that does not offer managed agents
//! answers [`CloudAgentError::Unsupported`] and the Cloud tab is hidden for it.

use std::sync::Arc;

use futures::future::BoxFuture;
use nocterm_hubs_model::HubId;

use crate::agent::CloudAgent;
use crate::error::CloudAgentError;
use crate::ids::{AgentId, PoolId};
use crate::personal::PersonalAgentDraft;
use crate::start::{PoolOffer, StartStep, StartedAgent};

/// A call of a Cloud agents port.
pub type CloudFuture<T> = BoxFuture<'static, Result<T, CloudAgentError>>;

/// Tells the caller which step a start is in.
pub type StepSink = Arc<dyn Fn(StartStep) + Send + Sync>;

/// Lossy change hints of a hub's agent list: a hint means "list again".
pub trait AgentHints: Send {
    /// Waits for the next hint; `None` when the hub ended the stream (subscribe again).
    fn next(&mut self) -> BoxFuture<'_, Result<Option<u64>, CloudAgentError>>;
}

/// A pool a personal agent may run on while the person is offline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolChoice {
    /// The pool.
    pub pool: PoolId,
    /// The pool's name.
    pub name: String,
    /// Whether the pool accepts personal keys.
    pub accepts_personal: bool,
}

/// Lists the agents of a hub and tells when the list changes.
pub trait CloudAgentCatalog: Send + Sync {
    /// Every agent the signed-in account may see on `hub`, in every scope.
    fn list(&self, hub: &HubId) -> CloudFuture<Vec<CloudAgent>>;

    /// Subscribes to change hints of the list.
    fn watch(&self, hub: &HubId) -> CloudFuture<Box<dyn AgentHints>>;

    /// The pools a personal agent may be sealed to.
    fn pools(&self, hub: &HubId) -> CloudFuture<Vec<PoolChoice>>;
}

/// What a start came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartOutcome {
    /// The seat is registered; a chat can be opened.
    Ready(StartedAgent),
    /// The pool's keys are not pinned on this device. Nothing was registered. Pin them with
    /// [`CloudAgentLauncher::pin_pool`] after the person compared the fingerprint, then start
    /// again: starting is idempotent.
    NeedsPoolTrust(PoolOffer),
}

/// Starts an agent: reserves its seat, checks the host's attestation under the pinned pool
/// key, registers the runner on this device and hands its grant to the host.
pub trait CloudAgentLauncher: Send + Sync {
    /// Starts (or resumes) the seat of `agent`, reporting each step to `steps`.
    ///
    /// A refusal is [`CloudAgentError::Refused`]; a pool whose pinned keys changed is
    /// [`CloudAgentError::PoolKeyChanged`]; nothing is registered in either case.
    fn start(&self, hub: &HubId, agent: &AgentId, steps: StepSink) -> CloudFuture<StartOutcome>;

    /// Pins the pool's keys as offered, after the person compared the fingerprint. Fails with
    /// [`CloudAgentError::PoolKeyChanged`] if the pool offers other keys than the ones shown.
    fn pin_pool(&self, hub: &HubId, offer: &PoolOffer) -> CloudFuture<()>;
}

/// What saving a personal agent came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The agent is saved.
    Saved(AgentId),
    /// The chosen pool's keys are not pinned: pin them, then save again.
    NeedsPoolTrust(PoolOffer),
}

/// Writes the signed-in person's own agents.
pub trait PersonalAgents: Send + Sync {
    /// Creates an agent (`existing` is `None`) or replaces the one with this id and version.
    /// The key and the definition leave the device only sealed to the chosen pool.
    fn save(
        &self,
        hub: &HubId,
        existing: Option<(AgentId, u64)>,
        draft: PersonalAgentDraft,
    ) -> CloudFuture<SaveOutcome>;

    /// Deletes the agent whose current version is `version`.
    fn delete(&self, hub: &HubId, agent: &AgentId, version: u64) -> CloudFuture<()>;
}

/// The address of the web panel of a hub, where organization and server agents are managed.
pub fn web_panel_url(hub: &HubId) -> String {
    format!("https://{}/", hub.address().host())
}
