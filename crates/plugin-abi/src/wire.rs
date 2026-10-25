//! What crosses the boundary. Postcard writes struct fields in order and enum variants by
//! index, so every type here is append-only: new variants go last, fields are never reordered
//! or removed, and nothing uses `skip_serializing_if`, `untagged` or `flatten`. The golden file
//! checked by xtask enforces it.

use alloc::{collections::BTreeMap, string::String, vec::Vec};

use nocterm_plugin_api::{
    ContributionContext, EffectOutcome, Effects, Event, HostError, LocalId, Observed, Snapshot,
    View,
    capability::{
        clock::ClockNow, monitor::Usage, notifications::OwnNotices, transfers::Counts,
        workspace::WorkspaceState,
    },
};
use nocterm_plugin_model::{SettingValue, SettingValues};
use serde::{Deserialize, Serialize};

/// What the host asks a guest to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Call {
    /// Draw a contribution.
    Render {
        local: LocalId,
        snapshot: WireSnapshot,
    },
    /// Answer an event of a contribution.
    Event {
        local: LocalId,
        event: Event,
        snapshot: WireSnapshot,
    },
}

/// What the guest answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Answer {
    /// The answer to [`Call::Render`].
    View(View),
    /// The answer to [`Call::Event`].
    Effects(Effects),
}

/// A guest's reply to one call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub answer: Answer,
    /// Log lines, at most [`MAX_LOG_LINES`](crate::MAX_LOG_LINES): the host refuses a reply with
    /// more, and sanitizes the lines it keeps.
    pub log: Vec<String>,
}

/// A [`Snapshot`] as ordered lists, so the bytes do not depend on a map implementation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireSnapshot {
    pub observations: Vec<(String, Result<WireObserved, HostError>)>,
    pub settings: Vec<(String, WireSetting)>,
    pub storage: Vec<(String, String)>,
    pub context: ContributionContext,
    pub outcomes: Vec<EffectOutcome>,
}

/// One-to-one with [`Observed`]. A new observation is appended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WireObserved {
    Clock(ClockNow),
    Workspace(WorkspaceState),
    Usage(Usage),
    Counts(Counts),
    Notices(OwnNotices),
}

/// One-to-one with [`SettingValue`]. A new kind is appended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireSetting {
    Bool(bool),
    Choice(String),
    Choices(Vec<String>),
    Number(i32),
    Text(String),
}

impl From<&Observed> for WireObserved {
    fn from(observed: &Observed) -> Self {
        match observed {
            Observed::Clock(data) => Self::Clock(*data),
            Observed::Workspace(data) => Self::Workspace(*data),
            Observed::Usage(data) => Self::Usage(*data),
            Observed::Counts(data) => Self::Counts(*data),
            Observed::Notices(data) => Self::Notices(data.clone()),
        }
    }
}

impl From<WireObserved> for Observed {
    fn from(wire: WireObserved) -> Self {
        match wire {
            WireObserved::Clock(data) => Self::Clock(data),
            WireObserved::Workspace(data) => Self::Workspace(data),
            WireObserved::Usage(data) => Self::Usage(data),
            WireObserved::Counts(data) => Self::Counts(data),
            WireObserved::Notices(data) => Self::Notices(data),
        }
    }
}

impl From<&SettingValue> for WireSetting {
    fn from(value: &SettingValue) -> Self {
        match value {
            SettingValue::Bool(on) => Self::Bool(*on),
            SettingValue::Choice(choice) => Self::Choice(choice.clone()),
            SettingValue::Choices(choices) => Self::Choices(choices.clone()),
            SettingValue::Number(number) => Self::Number(*number),
            SettingValue::Text(text) => Self::Text(text.clone()),
        }
    }
}

impl From<WireSetting> for SettingValue {
    fn from(wire: WireSetting) -> Self {
        match wire {
            WireSetting::Bool(on) => Self::Bool(on),
            WireSetting::Choice(choice) => Self::Choice(choice),
            WireSetting::Choices(choices) => Self::Choices(choices),
            WireSetting::Number(number) => Self::Number(number),
            WireSetting::Text(text) => Self::Text(text),
        }
    }
}

impl From<&Snapshot> for WireSnapshot {
    fn from(snapshot: &Snapshot) -> Self {
        Self {
            observations: snapshot
                .observations
                .iter()
                .map(|(id, observed)| {
                    (
                        id.clone(),
                        observed.as_ref().map(WireObserved::from).map_err(|e| *e),
                    )
                })
                .collect(),
            settings: snapshot
                .settings
                .iter()
                .map(|(key, value)| (String::from(key), WireSetting::from(value)))
                .collect(),
            storage: snapshot
                .storage
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            context: snapshot.context,
            outcomes: snapshot.outcomes.clone(),
        }
    }
}

impl From<WireSnapshot> for Snapshot {
    fn from(wire: WireSnapshot) -> Self {
        let mut settings = SettingValues::default();
        for (key, value) in wire.settings {
            settings.insert(key, SettingValue::from(value));
        }
        Self {
            observations: wire
                .observations
                .into_iter()
                .map(|(id, observed)| (id, observed.map(Observed::from)))
                .collect::<BTreeMap<_, _>>(),
            settings,
            storage: wire.storage.into_iter().collect(),
            context: wire.context,
            outcomes: wire.outcomes,
        }
    }
}
