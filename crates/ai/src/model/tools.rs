use super::{
    content::Content,
    extensions::Extensions,
    ids::{ProviderTerminalId, ToolId},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    ChangeMode,
    #[default]
    Other,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    #[default]
    Pending,
    Running,
    Succeeded,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolRecord {
    pub id: ToolId,
    pub title: String,
    pub name: Option<String>,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub output: Vec<ToolOutput>,
    pub locations: Vec<Location>,
    #[serde(with = "json_presence")]
    pub arguments: Option<Value>,
    #[serde(with = "json_presence")]
    pub result: Option<Value>,
    pub extensions: Option<Extensions>,
    pub claims: BridgeClaims,
    pub client_display: Option<ClientDisplay>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolPatch {
    pub id: ToolId,
    pub changes: ToolChanges,
    pub extensions: Option<Extensions>,
    pub claims: BridgeClaims,
}
/// None keeps the previous value; Some(empty) clears a collection. JSON null in
/// raw input/output decodes as absent under the current wire optional-field rules.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolChanges {
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub output: Option<Vec<ToolOutput>>,
    pub locations: Option<Vec<Location>>,
    #[serde(with = "json_presence")]
    pub arguments: Option<Value>,
    #[serde(with = "json_presence")]
    pub result: Option<Value>,
}
impl ToolRecord {
    pub fn apply(&mut self, patch: ToolPatch) -> bool {
        if self.id != patch.id {
            return false;
        }
        let changes = patch.changes;
        if let Some(value) = changes.kind {
            self.kind = value;
        }
        if let Some(value) = changes.status {
            self.status = value;
        }
        if let Some(value) = changes.title {
            self.title = value;
        }
        if let Some(value) = changes.name {
            self.name = Some(value);
        }
        if let Some(value) = changes.output {
            self.output = value;
        }
        if let Some(value) = changes.locations {
            self.locations = value;
        }
        if let Some(value) = changes.arguments {
            self.arguments = Some(value);
        }
        if let Some(value) = changes.result {
            self.result = Some(value);
        }
        self.claims.merge(patch.claims);
        true
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ToolOutput {
    Content {
        content: Box<Content>,
        extensions: Option<Extensions>,
    },
    FileDiff(FileDiff),
    TerminalReference {
        id: ProviderTerminalId,
        extensions: Option<Extensions>,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: PathBuf,
    pub previous: Option<String>,
    pub replacement: String,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub path: PathBuf,
    pub line: Option<u32>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Claim {
    #[default]
    Absent,
    Known {
        registration: String,
        tool: String,
    },
    Invalid,
}
impl Claim {
    fn merge(&mut self, incoming: Self) {
        match (&*self, &incoming) {
            (_, Self::Absent) => {}
            (Self::Absent, _) => *self = incoming,
            (current, next) if current == next => {}
            _ => *self = Self::Invalid,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeClaims {
    pub name: Claim,
    pub title: Claim,
    pub input: Claim,
    pub legacy_display_hint: Option<String>,
}
impl BridgeClaims {
    pub fn merge(&mut self, incoming: Self) {
        self.name.merge(incoming.name);
        self.title.merge(incoming.title);
        self.input.merge(incoming.input);
        if incoming.legacy_display_hint.is_some() {
            self.legacy_display_hint = incoming.legacy_display_hint;
        }
    }
    /// Evidence describes a row. It never grants permission to execute.
    pub fn consistent_identity(&self) -> Option<(&str, &str)> {
        let mut identity = None;
        for claim in [&self.name, &self.title, &self.input] {
            match claim {
                Claim::Absent => {}
                Claim::Invalid => return None,
                Claim::Known { registration, tool } => {
                    let next = (registration.as_str(), tool.as_str());
                    if identity.is_some_and(|current| current != next) {
                        return None;
                    }
                    identity = Some(next);
                }
            }
        }
        identity
    }
}

/// Durable presentation supplied by the client, never a restored capability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientDisplay {
    pub version: u8,
    pub tool: String,
    pub arguments: Value,
    pub destination: Option<String>,
    pub redacted: bool,
    pub outcome: Option<DisplayOutcome>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DisplayOutcome {
    Pending,
    Succeeded(Value),
    Failed(String),
}

/// The owned storage grammar preserves supplied JSON null without conflating
/// it with a missing optional field. Live adapter patches still follow the SDK's
/// optional-field decoding; this representation does not invent wire clears.
mod json_presence {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    #[derive(Serialize, Deserialize)]
    #[serde(tag = "presence", content = "value", rename_all = "snake_case")]
    enum Presence {
        Absent,
        Supplied(Value),
    }
    pub(super) fn serialize<S: serde::Serializer>(
        value: &Option<Value>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "presence", content = "value", rename_all = "snake_case")]
        enum BorrowedPresence<'a> {
            Absent,
            Supplied(&'a Value),
        }
        match value {
            None => BorrowedPresence::Absent,
            Some(value) => BorrowedPresence::Supplied(value),
        }
        .serialize(serializer)
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        decoder: D,
    ) -> Result<Option<Value>, D::Error> {
        Ok(match Presence::deserialize(decoder)? {
            Presence::Absent => None,
            Presence::Supplied(value) => Some(value),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_optional_json_roundtrips_literal_null_and_absence_separately() {
        let absent = ToolChanges::default();
        let supplied = ToolChanges {
            arguments: Some(Value::Null),
            ..ToolChanges::default()
        };
        for original in [absent, supplied] {
            let encoded = serde_json::to_vec(&original).unwrap();
            assert_eq!(
                serde_json::from_slice::<ToolChanges>(&encoded).unwrap(),
                original
            );
        }
    }
}
