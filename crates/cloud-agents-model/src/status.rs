//! What a person can do with an agent right now.

/// The state of an agent from this account's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentStatus {
    /// An instance is registered and a lease can start at once.
    Ready,
    /// The instance exists and is stopped; the next prompt wakes it.
    Sleeping,
    /// An instance is being provisioned or woken.
    Starting,
    /// A seat cannot be reserved now.
    NoCapacity,
    /// A personal agent needs its provider key.
    NeedsKey,
    /// The pool's key is not pinned on this device yet.
    NeedsPoolTrust,
    /// The agent is switched off.
    Disabled,
}

impl AgentStatus {
    /// Whether choosing the agent may start a chat. Trust and capacity are resolved by the
    /// start itself (the dialog, or a typed refusal), so only a missing key or a switched-off
    /// agent block the row.
    pub fn can_start(self) -> bool {
        !matches!(self, Self::Disabled | Self::NeedsKey)
    }

    /// The short chip text.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Sleeping => "Asleep",
            Self::Starting => "Starting",
            Self::NoCapacity => "No capacity",
            Self::NeedsKey => "Needs key",
            Self::NeedsPoolTrust => "Trust pool",
            Self::Disabled => "Off",
        }
    }

    /// Whether the chip should draw attention (everything that is not simply usable).
    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::NoCapacity | Self::NeedsKey | Self::NeedsPoolTrust | Self::Disabled
        )
    }

    /// Sort key: usable agents first, then the ones that wait, then the blocked ones.
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::Sleeping => 1,
            Self::Starting => 2,
            Self::NeedsPoolTrust => 3,
            Self::NoCapacity => 4,
            Self::NeedsKey => 5,
            Self::Disabled => 6,
        }
    }
}
