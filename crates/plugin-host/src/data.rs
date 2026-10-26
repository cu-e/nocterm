//! A plugin's own data: its setting values and its storage, in one bounded document.

use std::collections::BTreeMap;

use gpui::App;
use nocterm_plugin_api::capability::storage::{MAX_TOTAL, Remove, Set};
use nocterm_plugin_model::{PluginId, SettingValue, SettingValues};
use serde::{Deserialize, Serialize};

use crate::{DataBoundsError, DocumentError, PluginDataError};

/// Most serialized bytes of a plugin's setting values.
pub const MAX_VALUES_BYTES: usize = 4 * 1024;

/// One stored setting value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "kebab-case")]
pub enum StoredValue {
    Bool(bool),
    Choice(String),
    Choices(Vec<String>),
    Number(i32),
    Text(String),
}

impl From<&SettingValue> for StoredValue {
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

impl From<&StoredValue> for SettingValue {
    fn from(value: &StoredValue) -> Self {
        match value {
            StoredValue::Bool(on) => Self::Bool(*on),
            StoredValue::Choice(choice) => Self::Choice(choice.clone()),
            StoredValue::Choices(choices) => Self::Choices(choices.clone()),
            StoredValue::Number(number) => Self::Number(*number),
            StoredValue::Text(text) => Self::Text(text.clone()),
        }
    }
}

/// The file `<state>/plugins/<plugin-id>.json`: at most 4 KiB of values and 64 KiB of storage.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginDataDoc {
    pub values: BTreeMap<String, StoredValue>,
    pub storage: BTreeMap<String, String>,
}

impl PluginDataDoc {
    /// The values as the schema resolves them from.
    pub fn setting_values(&self) -> SettingValues {
        let mut values = SettingValues::default();
        for (key, value) in &self.values {
            values.insert(key.clone(), value.into());
        }
        values
    }

    /// This document with the given values.
    pub fn with_values(&self, values: &SettingValues) -> Result<Self, DataBoundsError> {
        let next = Self {
            values: values
                .iter()
                .map(|(key, value)| (key.to_owned(), value.into()))
                .collect(),
            storage: self.storage.clone(),
        };
        next.check()?;
        Ok(next)
    }

    /// This document with one storage entry set.
    pub fn with_set(&self, request: &Set) -> Result<Self, DataBoundsError> {
        request.check()?;
        let mut next = self.clone();
        next.storage
            .insert(request.key.clone(), request.value.clone());
        next.check()?;
        Ok(next)
    }

    /// This document without one storage entry.
    pub fn with_removed(&self, request: &Remove) -> Result<Self, DataBoundsError> {
        request.check()?;
        let mut next = self.clone();
        next.storage.remove(&request.key);
        Ok(next)
    }

    /// The bytes of the plugin's file.
    pub fn encode(&self) -> Result<Vec<u8>, DocumentError> {
        serde_json::to_vec_pretty(self).map_err(DocumentError::syntax)
    }

    /// The document in `bytes`, if it is one of this host and inside its bounds.
    pub fn decode(bytes: &[u8]) -> Result<Self, DocumentError> {
        let doc: Self = serde_json::from_slice(bytes).map_err(DocumentError::syntax)?;
        doc.check()?;
        Ok(doc)
    }

    /// Whether the document is inside its bounds.
    pub fn check(&self) -> Result<(), DataBoundsError> {
        let values = serde_json::to_vec(&self.values).map_or(usize::MAX, |bytes| bytes.len());
        if values > MAX_VALUES_BYTES {
            return Err(DataBoundsError::ValuesTooLarge {
                max: MAX_VALUES_BYTES,
            });
        }
        let storage: usize = self
            .storage
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum();
        if storage > MAX_TOTAL {
            return Err(DataBoundsError::StorageTooLarge { max: MAX_TOTAL });
        }
        Ok(())
    }
}

/// A change the host asks the port to persist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginDataChange {
    /// The document as it is now.
    Replace(PluginDataDoc),
    /// Delete the file.
    Remove,
}

/// Where plugin data is kept. Implemented by the application over its keyed persistence.
pub trait PluginData {
    /// The document of a plugin; defaults when none was written yet.
    fn load(&self, plugin: &PluginId, cx: &App) -> Result<PluginDataDoc, PluginDataError>;

    /// Queues the change for the plugin's file. Later changes to one plugin replace earlier ones.
    /// A refused change was not queued, so the caller must not treat the data as stored.
    fn change(
        &self,
        plugin: PluginId,
        change: PluginDataChange,
        cx: &mut App,
    ) -> Result<(), PluginDataError>;
}
