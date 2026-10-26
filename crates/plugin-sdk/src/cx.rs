//! What a plugin sees during one call.

use alloc::{string::String, vec::Vec};

use nocterm_plugin_abi::MAX_LOG_LINES;
use nocterm_plugin_api::{EffectOutcome, LocalId, Snapshot};
use nocterm_plugin_model::SettingValue;

use crate::observe::{Observable, ObserveError};

/// The context of one `render` or `on_event` call: the contribution it is for, the snapshot the
/// host assembled, and the log lines the reply will carry.
pub struct Cx<'a> {
    local: &'a LocalId,
    snapshot: &'a Snapshot,
    log: Vec<String>,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(local: &'a LocalId, snapshot: &'a Snapshot) -> Self {
        Self {
            local,
            snapshot,
            log: Vec::new(),
        }
    }

    pub(crate) fn into_log(self) -> Vec<String> {
        self.log
    }

    /// The local name of the contribution this call is for.
    pub fn local(&self) -> &LocalId {
        self.local
    }

    /// The whole snapshot, for what the typed reads do not cover.
    pub fn snapshot(&self) -> &Snapshot {
        self.snapshot
    }

    /// The observation of a capability the manifest declares.
    ///
    /// `Missing` means the manifest does not declare it; `Host` carries the refusal for an
    /// optional capability the user did not grant.
    pub fn observe<T: Observable>(&self) -> Result<&T::Value, ObserveError> {
        match self.snapshot.observation(T::ID) {
            None => Err(ObserveError::Missing),
            Some(Err(error)) => Err(ObserveError::Host(*error)),
            Some(Ok(observed)) => T::pick(observed).ok_or(ObserveError::Kind),
        }
    }

    /// The current value of one of the plugin's settings, resolved against its schema.
    pub fn setting(&self, key: &str) -> Option<&SettingValue> {
        self.snapshot.settings.get(key)
    }

    /// A value the plugin stored with `storage.set`.
    pub fn stored(&self, key: &str) -> Option<&str> {
        self.snapshot.storage.get(key).map(String::as_str)
    }

    /// What happened to the effects of the previous call.
    pub fn outcomes(&self) -> &[EffectOutcome] {
        &self.snapshot.outcomes
    }

    /// Adds a log line to the reply. The host keeps the first eight of a call, sanitizes them
    /// and writes them to the application log.
    pub fn log(&mut self, text: impl Into<String>) {
        if self.log.len() < MAX_LOG_LINES {
            self.log.push(text.into());
        }
    }
}
