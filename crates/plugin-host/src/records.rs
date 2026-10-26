//! What the host remembers between runs: which package each plugin is, what the user allowed
//! and whether it is on.

use gpui::{App, Task};
use nocterm_plugin_model::PluginId;
use serde::{Deserialize, Serialize};

use crate::{DocumentError, RecordsError};

/// Most plugin records a document keeps.
pub const MAX_RECORDS: usize = 64;
/// Most audit entries a document keeps; the oldest go first.
pub const MAX_AUDIT: usize = 64;

/// One installed plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginRecord {
    pub id: PluginId,
    /// The sha256 the user consented to, in lowercase hexadecimal.
    pub hash: String,
    pub enabled: bool,
    /// Whether the user accepted placing the plugin's contributions, for this hash.
    #[serde(default)]
    pub placeable: bool,
    /// The optional capabilities the user accepted at consent.
    #[serde(default)]
    pub granted_optional: Vec<String>,
}

/// What happened to a plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditAction {
    Installed,
    Replaced,
    Removed,
    Enabled,
    Disabled,
    Placeable,
    NotPlaceable,
    DataRemoved,
    Faulted,
}

/// One line of the audit ring.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEntry {
    /// Counts up over the life of the document, so the order survives trimming.
    pub seq: u64,
    pub plugin: PluginId,
    pub action: AuditAction,
}

/// The whole records document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecordsDoc {
    pub plugins: Vec<PluginRecord>,
    pub audit: Vec<AuditEntry>,
}

impl RecordsDoc {
    /// The bytes of the records file.
    pub fn encode(&self) -> Result<Vec<u8>, DocumentError> {
        serde_json::to_vec_pretty(self).map_err(DocumentError::syntax)
    }

    /// The document in `bytes`, trimmed to its limits.
    pub fn decode(bytes: &[u8]) -> Result<Self, DocumentError> {
        let doc: Self = serde_json::from_slice(bytes).map_err(DocumentError::syntax)?;
        Ok(doc.bounded())
    }

    /// Drops records whose id is not a user plugin id (nothing could ever remove them), repeated
    /// ids and entries past the limits, as a hand-edited file may hold them.
    pub fn bounded(mut self) -> Self {
        let mut seen = std::collections::BTreeSet::new();
        self.plugins
            .retain(|record| record.id.is_user() && record.id.check().is_ok());
        self.plugins.retain(|record| seen.insert(record.id.clone()));
        self.plugins.truncate(MAX_RECORDS);
        let excess = self.audit.len().saturating_sub(MAX_AUDIT);
        self.audit.drain(..excess);
        self
    }
}

/// Appends an entry to an audit ring and trims its oldest entries.
pub(crate) fn push_audit(audit: &mut Vec<AuditEntry>, plugin: &PluginId, action: AuditAction) {
    let seq = audit.last().map_or(0, |entry| entry.seq + 1);
    audit.push(AuditEntry {
        seq,
        plugin: plugin.clone(),
        action,
    });
    let excess = audit.len().saturating_sub(MAX_AUDIT);
    audit.drain(..excess);
}

/// Where the records are kept. Implemented by the application over its persistence service.
pub trait PluginRecords {
    /// The document last saved, or an empty one.
    fn load(&self, cx: &App) -> RecordsDoc;

    /// Replaces the document. The task ends when the write was published.
    fn replace(&self, doc: RecordsDoc, cx: &mut App) -> Task<Result<(), RecordsError>>;
}
