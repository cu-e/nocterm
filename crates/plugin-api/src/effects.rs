//! What a plugin asks the host to do, and what the host tells it happened.

use alloc::{collections::BTreeSet, string::String, vec::Vec};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    HostError, LimitError, LocalId,
    capability::{
        Capability,
        notifications::{Post, PostNotice},
        panels::{SetBadge, SetOpen, SetPanelBadge, SetPanelOpen},
        storage::{Remove, Set, StoreRemove, StoreSet},
    },
    error::key_syntax,
};

/// Most effects one call may return.
pub const MAX_EFFECTS: usize = 8;
/// Longest correlation key.
pub const MAX_EFFECT_KEY: usize = 32;

/// Something the user did to a plugin's view, or a change the plugin may react to.
///
/// No key, character or modifier data reaches the plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Event {
    /// A button was activated by pointer, or by Enter or Space on a focused host control.
    Activated {
        key: String,
    },
    Toggled {
        key: String,
        on: bool,
    },
    VisibilityChanged {
        visible: bool,
    },
    SettingsChanged,
}

/// One thing a plugin asks for. Each variant is one effect capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    #[serde(rename = "notifications.post")]
    NotificationsPost(Post),
    #[serde(rename = "panels.set-open")]
    PanelsSetOpen(SetOpen),
    #[serde(rename = "panels.set-badge")]
    PanelsSetBadge(SetBadge),
    #[serde(rename = "storage.set")]
    StorageSet(Set),
    #[serde(rename = "storage.remove")]
    StorageRemove(Remove),
}

impl Effect {
    /// The id of the capability the effect needs.
    pub fn capability(&self) -> &'static str {
        match self {
            Self::NotificationsPost(_) => PostNotice::ID,
            Self::PanelsSetOpen(_) => SetPanelOpen::ID,
            Self::PanelsSetBadge(_) => SetPanelBadge::ID,
            Self::StorageSet(_) => StoreSet::ID,
            Self::StorageRemove(_) => StoreRemove::ID,
        }
    }

    /// The own contribution the effect names, if it names one.
    pub fn local(&self) -> Option<&LocalId> {
        match self {
            Self::PanelsSetOpen(request) => Some(&request.local),
            Self::PanelsSetBadge(request) => Some(&request.local),
            _ => None,
        }
    }

    fn check(&self) -> Result<(), LimitError> {
        match self {
            Self::NotificationsPost(request) => request.check(),
            Self::StorageSet(request) => request.check(),
            Self::StorageRemove(request) => request.check(),
            Self::PanelsSetOpen(_) | Self::PanelsSetBadge(_) => Ok(()),
        }
    }
}

/// An effect and the key its outcome comes back under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EffectRequest {
    #[serde(default)]
    pub key: Option<String>,
    pub effect: Effect,
}

/// What `on_event` returns: at most [`MAX_EFFECTS`] requests.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Effects(pub Vec<EffectRequest>);

/// Effects the host drops whole.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EffectsError {
    #[error("a call returns at most {MAX_EFFECTS} effects")]
    TooMany,
    #[error("key `{key}` is used by two effects")]
    DuplicateKey { key: String },
    #[error("effect {index}: {source}")]
    Limit {
        index: usize,
        #[source]
        source: LimitError,
    },
}

impl Effects {
    /// Checks the count, the keys and the limits of every effect.
    pub fn check(&self) -> Result<(), EffectsError> {
        if self.0.len() > MAX_EFFECTS {
            return Err(EffectsError::TooMany);
        }
        let mut keys = BTreeSet::new();
        for (index, request) in self.0.iter().enumerate() {
            let limit = |source| EffectsError::Limit { index, source };
            if let Some(key) = &request.key {
                key_syntax(key, MAX_EFFECT_KEY, "key").map_err(limit)?;
                if !keys.insert(key.as_str()) {
                    return Err(EffectsError::DuplicateKey { key: key.clone() });
                }
            }
            request.effect.check().map_err(limit)?;
        }
        Ok(())
    }
}

/// What happened to an effect of the previous call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EffectOutcome {
    pub key: Option<String>,
    pub result: Result<(), HostError>,
}
