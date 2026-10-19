//! Tool rows. `ToolRecord` and its children are stored in saved transcripts, so their
//! JSON shape (names, field order, omitted defaults, lenient decoding) is a compatibility contract.
use super::{
    content::Content,
    extensions::Extensions,
    ids::{ProviderTerminalId, ToolId},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::{DefaultOnError, VecSkipError, serde_as, skip_serializing_none};
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
    #[serde(rename = "switch_mode")]
    ChangeMode,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    #[default]
    Pending,
    #[serde(rename = "in_progress")]
    Running,
    #[serde(rename = "completed")]
    Succeeded,
    Failed,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRecord {
    #[serde(rename = "toolCallId")]
    pub id: ToolId,
    pub title: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub name: Option<String>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: ToolKind,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, skip_serializing_if = "is_default")]
    pub status: ToolStatus,
    #[serde_as(deserialize_as = "DefaultOnError<VecSkipError<_>>")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolContent>,
    #[serde_as(deserialize_as = "DefaultOnError<VecSkipError<_>>")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locations: Vec<Location>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub raw_input: Option<Value>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub raw_output: Option<Value>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}
impl ToolRecord {
    pub fn new(id: impl Into<ToolId>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            name: None,
            kind: ToolKind::default(),
            status: ToolStatus::default(),
            content: Vec::new(),
            locations: Vec::new(),
            raw_input: None,
            raw_output: None,
            meta: None,
        }
    }
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
    pub fn kind(mut self, kind: ToolKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn status(mut self, status: ToolStatus) -> Self {
        self.status = status;
        self
    }
    pub fn content(mut self, content: Vec<ToolContent>) -> Self {
        self.content = content;
        self
    }
    pub fn raw_input(mut self, value: impl Into<Option<Value>>) -> Self {
        self.raw_input = value.into();
        self
    }
    pub fn raw_output(mut self, value: impl Into<Option<Value>>) -> Self {
        self.raw_output = value.into();
        self
    }
    pub fn meta(mut self, meta: impl Into<Option<Extensions>>) -> Self {
        self.meta = meta.into();
        self
    }
    /// Absent changes keep the current value; an empty list clears it.
    pub fn apply(&mut self, changes: ToolChanges) {
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
        if let Some(value) = changes.content {
            self.content = value;
        }
        if let Some(value) = changes.locations {
            self.locations = value;
        }
        if let Some(value) = changes.raw_input {
            self.raw_input = Some(value);
        }
        if let Some(value) = changes.raw_output {
            self.raw_output = Some(value);
        }
    }
}

/// A sparse update of one tool row.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolPatch {
    pub id: ToolId,
    pub changes: ToolChanges,
    pub meta: Option<Extensions>,
}
impl ToolPatch {
    pub fn new(id: impl Into<ToolId>, changes: ToolChanges) -> Self {
        Self {
            id: id.into(),
            changes,
            meta: None,
        }
    }
}

/// None keeps the previous value; Some(empty) clears a collection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolChanges {
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub content: Option<Vec<ToolContent>>,
    pub locations: Option<Vec<Location>>,
    pub raw_input: Option<Value>,
    pub raw_output: Option<Value>,
}
impl ToolChanges {
    pub fn kind(mut self, kind: ToolKind) -> Self {
        self.kind = Some(kind);
        self
    }
    pub fn status(mut self, status: ToolStatus) -> Self {
        self.status = Some(status);
        self
    }
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn content(mut self, content: Vec<ToolContent>) -> Self {
        self.content = Some(content);
        self
    }
    pub fn raw_input(mut self, value: Value) -> Self {
        self.raw_input = Some(value);
        self
    }
    pub fn raw_output(mut self, value: Value) -> Self {
        self.raw_output = Some(value);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    Content(Box<ContentOutput>),
    Diff(FileDiff),
    Terminal(TerminalReference),
}
impl From<Content> for ToolContent {
    fn from(content: Content) -> Self {
        Self::Content(Box::new(ContentOutput {
            content,
            meta: None,
        }))
    }
}
impl From<&str> for ToolContent {
    fn from(text: &str) -> Self {
        Content::from(text).into()
    }
}
impl From<String> for ToolContent {
    fn from(text: String) -> Self {
        Content::from(text).into()
    }
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentOutput {
    pub content: Content,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: PathBuf,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub old_text: Option<String>,
    pub new_text: String,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalReference {
    pub terminal_id: ProviderTerminalId,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}

#[serde_as]
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub path: PathBuf,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub line: Option<u32>,
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default, rename = "_meta")]
    pub meta: Option<Extensions>,
}
