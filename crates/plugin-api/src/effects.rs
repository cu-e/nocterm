//! What a plugin asks the host to do, and what the host tells it happened.

use alloc::{collections::BTreeSet, string::String, vec::Vec};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    HostError, LimitError, LocalId,
    capability::{
        Capability,
        files::{FileOp, FileReply, ServeFiles},
        net::{NetRequest, Request},
        notifications::{Post, PostNotice},
        panels::{SetBadge, SetOpen, SetPanelBadge, SetPanelOpen},
        storage::{Remove, Set, StoreRemove, StoreSet},
        terminals::{Bar, Open, OpenTerminal, ShowTerminalBar, Write, WriteTerminal},
        timers::{SetTimer, Timer},
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
    /// The answer to the `net.request` effect with this key. `status` is the HTTP status, or 0
    /// when no answer came (the body then says why, in the host's words). A body longer than
    /// the response limit is cut there; bytes that are not UTF-8 are replaced.
    Response {
        key: String,
        status: u16,
        body: String,
    },
    /// The `timers.set` effect with this key ran out.
    Timer {
        key: String,
    },
    /// A line was entered in one of the plugin's terminals, by the user or an AI chat.
    TerminalLine {
        terminal: String,
        line: String,
    },
    /// The user closed one of the plugin's terminals.
    TerminalClosed {
        terminal: String,
    },
    /// The file browser or an AI chat asks the files of one of the plugin's terminals, opened
    /// with `files`; the plugin answers with a `terminals.files` effect under `request`.
    FileRequest {
        terminal: String,
        request: u32,
        op: FileOp,
    },
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
    #[serde(rename = "net.request")]
    NetRequest(Request),
    #[serde(rename = "timers.set")]
    TimersSet(SetTimer),
    #[serde(rename = "terminals.open")]
    TerminalsOpen(Open),
    #[serde(rename = "terminals.write")]
    TerminalsWrite(Write),
    #[serde(rename = "terminals.files")]
    TerminalsFiles(FileReply),
    #[serde(rename = "terminals.bar")]
    TerminalsBar(Bar),
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
            Self::NetRequest(_) => NetRequest::ID,
            Self::TimersSet(_) => Timer::ID,
            Self::TerminalsOpen(_) => OpenTerminal::ID,
            Self::TerminalsWrite(_) => WriteTerminal::ID,
            Self::TerminalsFiles(_) => ServeFiles::ID,
            Self::TerminalsBar(_) => ShowTerminalBar::ID,
        }
    }

    /// Whether the outcome comes back as an event under the request's key, which it then needs.
    pub fn needs_key(&self) -> bool {
        matches!(self, Self::NetRequest(_) | Self::TimersSet(_))
    }

    /// The own contribution the effect names, if it names one.
    pub fn local(&self) -> Option<&LocalId> {
        match self {
            Self::PanelsSetOpen(request) => Some(&request.local),
            Self::PanelsSetBadge(request) => Some(&request.local),
            Self::TerminalsOpen(request) => Some(&request.local),
            _ => None,
        }
    }

    fn check(&self) -> Result<(), LimitError> {
        match self {
            Self::NotificationsPost(request) => request.check(),
            Self::StorageSet(request) => request.check(),
            Self::StorageRemove(request) => request.check(),
            Self::NetRequest(request) => request.check(),
            Self::TimersSet(request) => request.check(),
            Self::TerminalsOpen(request) => request.check(),
            Self::TerminalsWrite(request) => request.check(),
            Self::TerminalsFiles(request) => request.check(),
            Self::TerminalsBar(request) => request.check(),
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
    #[error("effect {index} answers with an event and needs a key")]
    MissingKey { index: usize },
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
            if request.key.is_none() && request.effect.needs_key() {
                return Err(EffectsError::MissingKey { index });
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
