//! Why a Cloud agents call failed.

use nocterm_hubs_model::HubError;

use crate::start::{Refusal, SetupProblem};

/// A catalog or launcher call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CloudAgentError {
    /// The hub does not offer managed agents; the tab is hidden for it.
    #[error("this hub does not offer managed agents")]
    Unsupported,
    /// This device holds no credentials for the hub.
    #[error("this device is not signed in to the hub")]
    NotSignedIn,
    /// The hub refused to reserve a seat for a typed reason.
    #[error("the hub refused the request: {0}")]
    Refused(Refusal),
    /// The pool offers other keys than the ones pinned on this device: a substitution until
    /// proven otherwise. Nothing was registered.
    #[error("the pool key changed: pinned {pinned}, offered {offered}")]
    PoolKeyChanged {
        /// Fingerprint of the pinned keys.
        pinned: String,
        /// Fingerprint of the keys now offered.
        offered: String,
    },
    /// The person did not trust the pool.
    #[error("the pool was not trusted")]
    PoolNotTrusted,
    /// The host's runner setup cannot be trusted or imported.
    #[error("the runner setup cannot be trusted: {0}")]
    Setup(SetupProblem),
    /// The seat did not become usable.
    #[error("the host could not create the agent's runner")]
    ProvisioningFailed,
    /// The hub did not finish provisioning in time.
    #[error("the host took too long to prepare the agent")]
    TimedOut,
    /// Agents that work only while a device of the person is online need an own runner, which
    /// this build cannot set up yet.
    #[error("agents that work only while you are online are not available yet")]
    OnlineOnlyUnavailable,
    /// No runner of the person's own is registered on this device to lease the key to.
    #[error("no runner of yours is registered on this device")]
    NoOwnRunner,
    /// This device holds no key for the agent, so nothing can be leased.
    #[error("this device holds no key for the agent")]
    NoOwnKey,
    /// The lease of the agent's key is not tracked: it ended or was never started.
    #[error("the lease of the agent's key has ended")]
    LeaseEnded,
    /// The agent changed on the hub meanwhile.
    #[error("the agent changed on the hub")]
    Conflict,
    /// The account or hub of the call ended meanwhile.
    #[error("the account or hub changed while the call was running")]
    Stale,
    /// The input is not acceptable; nothing was sent.
    #[error(transparent)]
    Invalid(#[from] crate::personal::DraftError),
    /// The device refused to import or register the runner.
    #[error("the runner could not be registered on this device")]
    Registration(#[source] nocterm_core::error::SharedCause),
    /// Any other failure of the hub connection.
    #[error(transparent)]
    Hub(#[from] HubError),
}

impl CloudAgentError {
    /// Whether a person could retry the same action later without changing anything.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Refused(Refusal::HostCapacity) | Self::TimedOut | Self::Hub(_)
        )
    }
}
