//! Sections for this crate's tests; the real ones live with their owners.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{SettingsSection, clamp_f32};

/// How terminals look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Terminal {
    /// Font size in pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    /// Shape of the cursor.
    pub cursor_shape: String,
    /// Copy text as soon as it is selected.
    pub copy_on_select: bool,
}

impl Default for Terminal {
    fn default() -> Self {
        Self {
            font_size: None,
            cursor_shape: "block".into(),
            copy_on_select: false,
        }
    }
}

impl SettingsSection for Terminal {
    const KEY: &'static str = "terminal";

    fn sanitize(&mut self) {
        self.font_size = self.font_size.and_then(|size| clamp_f32(size, 6.0..=72.0));
    }
}

/// The host monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Monitor {
    /// Seconds between samples.
    pub interval_secs: u32,
}

impl Default for Monitor {
    fn default() -> Self {
        Self { interval_secs: 2 }
    }
}

impl SettingsSection for Monitor {
    const KEY: &'static str = "monitor";
}

/// Agents by id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Agents {
    pub agents: BTreeMap<String, Agent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Agent {
    #[serde(skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            enabled: true,
            command: None,
        }
    }
}

fn is_true(value: &bool) -> bool {
    *value
}

impl SettingsSection for Agents {
    const KEY: &'static str = "ai";
}

/// A document with every fixture section registered.
pub(crate) fn registered(mut document: crate::SettingsDocument) -> crate::SettingsDocument {
    document.register::<Terminal>();
    document.register::<Monitor>();
    document.register::<Agents>();
    document
}
