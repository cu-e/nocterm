use super::json::OwnedJson;
use super::{
    content::Content,
    extensions::Extensions,
    ids::{ProviderTerminalId, ToolId},
};
use serde::{Deserialize, Serialize};
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
impl ToolStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Running => "InProgress",
            Self::Succeeded => "Completed",
            Self::Failed => "Failed",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolRecord {
    pub id: ToolId,
    pub title: String,
    pub name: Option<String>,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub output: Vec<ToolOutput>,
    pub locations: Vec<Location>,
    #[serde(with = "json_presence")]
    pub arguments: Option<OwnedJson>,
    #[serde(with = "json_presence")]
    pub result: Option<OwnedJson>,
    pub extensions: Option<Extensions>,
    #[serde(skip)]
    pub claims: BridgeClaims,
    #[serde(skip)]
    pub literal: super::literal::LiteralEvidence,
    #[serde(with = "shared_display")]
    pub client_display: Option<super::shared_display::SharedDisplay>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolPatch {
    pub id: ToolId,
    pub changes: ToolChanges,
    pub extensions: Option<Extensions>,
    #[serde(skip)]
    pub claims: BridgeClaims,
    #[serde(skip)]
    pub literal: super::literal::LiteralEvidence,
}
/// None keeps the previous value; Some(empty) clears a collection. JSON null in
/// raw input/output decodes as absent under the current wire optional-field rules.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ToolChanges {
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub output: Option<Vec<ToolOutput>>,
    pub locations: Option<Vec<Location>>,
    #[serde(with = "json_presence")]
    pub arguments: Option<OwnedJson>,
    #[serde(with = "json_presence")]
    pub result: Option<OwnedJson>,
}
impl ToolRecord {
    pub(crate) fn adopt_retention(
        &self,
        authority: &super::retention::RetentionAuthority,
    ) -> Result<(), super::retention::RetentionError> {
        self.id.adopt_retention(authority)?;
        for output in &self.output {
            if let ToolOutput::TerminalReference { id, .. } = output {
                id.adopt_retention(authority)?;
            }
        }
        Ok(())
    }

    pub fn apply(&mut self, patch: ToolPatch) -> bool {
        if self.id != patch.id {
            return false;
        }
        let changes = patch.changes;
        self.literal.apply(
            patch.literal,
            changes.name.is_some(),
            changes.title.is_some(),
            changes.arguments.is_some(),
        );
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
#[derive(Clone, Debug, PartialEq, Serialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FileDiff {
    pub path: PathBuf,
    pub previous: Option<String>,
    pub replacement: String,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClientDisplay {
    pub version: u8,
    pub tool: String,
    pub arguments: OwnedJson,
    pub destination: super::controls::Presence<String>,
    pub redacted: super::controls::Presence<bool>,
    pub outcome: super::controls::Presence<DisplayOutcome>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum DisplayOutcome {
    Pending,
    #[serde(rename = "Ok")]
    Succeeded(OwnedJson),
    #[serde(rename = "Err")]
    Failed(String),
}

/// The owned storage grammar preserves supplied JSON null without conflating
/// it with a missing optional field. Live adapter patches still follow the SDK's
/// optional-field decoding; this representation does not invent wire clears.
mod json_presence {
    use super::super::json::OwnedJson;
    use serde::Serialize;
    pub(super) fn serialize<S: serde::Serializer>(
        value: &Option<OwnedJson>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "presence", content = "value", rename_all = "snake_case")]
        enum BorrowedPresence<'a> {
            Absent,
            Supplied(&'a OwnedJson),
        }
        match value {
            None => BorrowedPresence::Absent,
            Some(value) => BorrowedPresence::Supplied(value),
        }
        .serialize(serializer)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_optional_json_roundtrips_literal_null_and_absence_separately() {
        let authority = crate::model::retention::RetentionAuthority::new(usize::MAX).unwrap();
        let absent = ToolChanges::default();
        let supplied = ToolChanges {
            arguments: Some(
                OwnedJson::from_untracked(&authority, serde_json::Value::Null).unwrap(),
            ),
            ..ToolChanges::default()
        };
        for original in [absent, supplied] {
            let encoded = serde_json::to_vec(&original).unwrap();
            assert_eq!(
                <ToolChanges as crate::model::decode::Decode>::decode(
                    &authority,
                    &mut serde_json::Deserializer::from_slice(&encoded)
                )
                .unwrap(),
                original
            );
        }
    }
}

mod shared_display {
    use crate::model::shared_display::SharedDisplay;
    use serde::Serialize;
    pub(super) fn serialize<S: serde::Serializer>(
        value: &Option<SharedDisplay>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_deref().serialize(serializer)
    }
}
