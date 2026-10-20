use super::AgentLaunch;
use std::collections::BTreeMap;

/// The pinned command a known agent starts with when settings do not override it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchDefaults {
    command: &'static str,
    args: &'static [&'static str],
    inherit_env: &'static [&'static str],
}

impl LaunchDefaults {
    /// `inherit_env` names the user's variables the agent may see, such as its API key.
    pub const fn new(
        command: &'static str,
        args: &'static [&'static str],
        inherit_env: &'static [&'static str],
    ) -> Self {
        Self {
            command,
            args,
            inherit_env,
        }
    }
    pub fn command(&self) -> &'static str {
        self.command
    }
    pub fn args(&self) -> &'static [&'static str] {
        self.args
    }
    pub fn inherit_env(&self) -> &'static [&'static str] {
        self.inherit_env
    }
}

/// Where an agent attaches plan limits to its ACP usage updates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UsageMeta {
    /// The agent reports no plan limits with usage.
    #[default]
    None,
    /// A rate-limit record (`rateLimitType`, `utilization`, `resetsAt`, `status`)
    /// under `key` of the usage update's `_meta`.
    RateLimitRecord { key: &'static str },
}

/// How nocterm reads an agent's plan limits outside the protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AccountUsage {
    /// Limits arrive with usage updates only.
    #[default]
    None,
    /// The agent keeps an OAuth sign-in whose account reports limits.
    OAuthAccount,
    /// The agent logs limits to session files in its home.
    SessionLog,
}

/// How an agent reports plan limits, and what to say while none are known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UsageProfile {
    meta: UsageMeta,
    account: AccountUsage,
    empty_hint: Option<&'static str>,
}

impl UsageProfile {
    pub const fn new(meta: UsageMeta, account: AccountUsage) -> Self {
        Self {
            meta,
            account,
            empty_hint: None,
        }
    }
    /// Shown in the usage report while no plan limits are known.
    pub const fn with_empty_hint(mut self, hint: &'static str) -> Self {
        self.empty_hint = Some(hint);
        self
    }
    pub fn meta(&self) -> UsageMeta {
        self.meta
    }
    pub fn account(&self) -> AccountUsage {
        self.account
    }
    pub fn empty_hint(&self) -> Option<&'static str> {
        self.empty_hint
    }
}

/// An agent's brand mark: a one-colour SVG served at `asset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentMark {
    asset: &'static str,
    svg: &'static [u8],
    tint: Option<u32>,
}

impl AgentMark {
    /// `asset` is the path views request the SVG under.
    pub const fn new(asset: &'static str, svg: &'static [u8]) -> Self {
        Self {
            asset,
            svg,
            tint: None,
        }
    }
    /// The brand colour as `0xRRGGBB`; unset marks use the text colour.
    pub const fn tinted(mut self, rgb: u32) -> Self {
        self.tint = Some(rgb);
        self
    }
    pub fn asset(&self) -> &'static str {
        self.asset
    }
    pub fn svg(&self) -> &'static [u8] {
        self.svg
    }
    pub fn tint(&self) -> Option<u32> {
        self.tint
    }
}

/// What nocterm knows about one agent beyond plain ACP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentProfile {
    id: &'static str,
    name: &'static str,
    launch: LaunchDefaults,
    markers: &'static [&'static str],
    state: &'static [&'static str],
    usage: UsageProfile,
    mark: Option<AgentMark>,
}

impl AgentProfile {
    pub const fn new(id: &'static str, name: &'static str, launch: LaunchDefaults) -> Self {
        Self {
            id,
            name,
            launch,
            markers: &[],
            state: &[],
            usage: UsageProfile::new(UsageMeta::None, AccountUsage::None),
            mark: None,
        }
    }
    /// Launches whose command or arguments contain one of `markers` run this
    /// agent under another id, such as a custom command wrapping its adapter.
    pub const fn recognized_by(mut self, markers: &'static [&'static str]) -> Self {
        self.markers = markers;
        self
    }
    /// Paths relative to the home directory the agent writes its sign-in,
    /// history and caches to; an isolated agent may write them.
    pub const fn writes_home(mut self, paths: &'static [&'static str]) -> Self {
        self.state = paths;
        self
    }
    pub const fn with_usage(mut self, usage: UsageProfile) -> Self {
        self.usage = usage;
        self
    }
    pub const fn with_mark(mut self, mark: AgentMark) -> Self {
        self.mark = Some(mark);
        self
    }

    pub fn id(&self) -> &'static str {
        self.id
    }
    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn launch_defaults(&self) -> LaunchDefaults {
        self.launch
    }
    pub fn markers(&self) -> &'static [&'static str] {
        self.markers
    }
    pub fn home_state(&self) -> &'static [&'static str] {
        self.state
    }
    pub fn usage(&self) -> UsageProfile {
        self.usage
    }
    pub fn mark(&self) -> Option<AgentMark> {
        self.mark
    }

    /// Whether `launch` runs this agent: by its id, or by a marker in the command.
    pub fn recognizes(&self, launch: &AgentLaunch) -> bool {
        launch.id == self.id || self.markers.iter().any(|marker| launch.mentions(marker))
    }

    /// The launch used when settings do not override it.
    pub fn default_launch(&self) -> AgentLaunch {
        AgentLaunch {
            id: self.id.into(),
            name: self.name.into(),
            command: self.launch.command.into(),
            args: self.launch.args.iter().map(|&arg| arg.into()).collect(),
            env: BTreeMap::new(),
            inherit_env: self
                .launch
                .inherit_env
                .iter()
                .map(|&name| name.into())
                .collect(),
        }
    }
}
