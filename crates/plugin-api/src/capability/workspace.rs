//! `workspace.kind`.

use alloc::string::ToString;
use serde::{Deserialize, Serialize};

use super::{Capability, Fields, Value, ValueType, sealed};

/// What the window's workspace is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceKind {
    Local,
    Remote,
    Container,
    Folder,
}

impl WorkspaceKind {
    /// Every kind, in declaration order.
    pub const ALL: [Self; 4] = [Self::Local, Self::Remote, Self::Container, Self::Folder];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Container => "container",
            Self::Folder => "folder",
        }
    }
}

/// The kind of workspace and whether it is connected: no title, host, user, path or tab count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct WorkspaceState {
    pub kind: WorkspaceKind,
    pub connected: bool,
}

impl Fields for WorkspaceState {
    const FIELDS: &'static [(&'static str, ValueType)] =
        &[("kind", ValueType::Text), ("connected", ValueType::Bool)];

    fn field(&self, name: &str) -> Option<Value> {
        match name {
            "kind" => Some(Value::Text(self.kind.name().to_string())),
            "connected" => Some(Value::Bool(self.connected)),
            _ => None,
        }
    }
}

/// Marker of `workspace.kind`.
pub struct Kind;

impl sealed::Sealed for Kind {}

impl Capability for Kind {
    const ID: &'static str = "workspace.kind";
    type Value = WorkspaceState;
}
