//! What the host tells a plugin each time it asks for a view or an event answer.

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use nocterm_plugin_model::SettingValues;

use crate::{
    ContributionContext, EffectOutcome, HostError,
    capability::{
        Capability, Fields, Value, clock::ClockNow, monitor::Usage, notifications::OwnNotices,
        transfers::Counts, workspace::WorkspaceState,
    },
};

/// The data one observation carries.
#[derive(Clone, Debug, PartialEq)]
pub enum Observed {
    Clock(ClockNow),
    Workspace(WorkspaceState),
    Usage(Usage),
    Counts(Counts),
    Notices(OwnNotices),
}

impl Observed {
    /// The id of the capability that produced it.
    pub fn capability(&self) -> &'static str {
        match self {
            Self::Clock(_) => crate::capability::clock::Now::ID,
            Self::Workspace(_) => crate::capability::workspace::Kind::ID,
            Self::Usage(_) => crate::capability::monitor::UsageNow::ID,
            Self::Counts(_) => crate::capability::transfers::CountsNow::ID,
            Self::Notices(_) => crate::capability::notifications::Own::ID,
        }
    }

    /// A field of the data, for a template.
    pub fn field(&self, name: &str) -> Option<Value> {
        match self {
            Self::Clock(data) => data.field(name),
            Self::Workspace(data) => data.field(name),
            Self::Usage(data) => data.field(name),
            Self::Counts(data) => data.field(name),
            Self::Notices(_) => None,
        }
    }

    /// The whole data as one value, when it is a single instant.
    pub fn whole(&self) -> Option<Value> {
        match self {
            Self::Clock(clock) => Some(Value::Time {
                unix_minutes: clock.unix_minutes,
                utc_offset_seconds: clock.utc_offset_seconds,
            }),
            _ => None,
        }
    }
}

/// The input of `render` and `on_event`.
///
/// It carries no window id. Observations the plugin did not declare are absent; an optional
/// capability the user did not grant is `Err(HostError::Unavailable)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// Granted observations by capability id.
    pub observations: BTreeMap<String, Result<Observed, HostError>>,
    /// The plugin's setting values, resolved against its schema.
    pub settings: SettingValues,
    /// The plugin's own storage, at most 64 KiB.
    pub storage: BTreeMap<String, String>,
    pub context: ContributionContext,
    /// What happened to the effects of the previous call, by their keys.
    pub outcomes: Vec<EffectOutcome>,
}

impl Snapshot {
    /// An empty snapshot for `context`.
    pub fn new(context: ContributionContext) -> Self {
        Self {
            observations: BTreeMap::new(),
            settings: SettingValues::default(),
            storage: BTreeMap::new(),
            context,
            outcomes: Vec::new(),
        }
    }

    /// The observation of a capability, as the template interpreter and a guest read it.
    pub fn observation(&self, capability: &str) -> Option<&Result<Observed, HostError>> {
        self.observations.get(capability)
    }
}
