use super::{
    extensions::Extensions,
    ids::{ConfigChoiceId, ConfigGroupId, ConfigId, ModeId},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfigOption {
    pub id: ConfigId,
    pub name: String,
    pub description: Option<String>,
    pub category: Option<ConfigCategory>,
    pub kind: ConfigKind,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ConfigKind {
    Boolean(bool),
    Select {
        current: ConfigChoiceId,
        choices: SelectChoices,
    },
    Unsupported {
        label: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SelectChoices {
    Flat(Vec<ConfigChoice>),
    Groups(Vec<ConfigGroup>),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfigChoice {
    pub id: ConfigChoiceId,
    pub name: String,
    pub description: Option<String>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConfigGroup {
    pub id: ConfigGroupId,
    pub name: String,
    pub choices: Vec<ConfigChoice>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigCategory {
    Mode,
    Model,
    ModelConfig,
    ThoughtLevel,
    Other(String),
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Modes {
    pub current: ModeId,
    pub choices: Vec<ModeChoice>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModeChoice {
    pub id: ModeId,
    pub name: String,
    pub description: Option<String>,
    pub extensions: Option<Extensions>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Change<T> {
    #[default]
    Keep,
    Set(T),
    Clear,
}

/// Supplied value provenance is semantic data, independent of transport field spelling.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "presence", content = "value", rename_all = "snake_case")]
pub enum Presence<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}
impl<T> Presence<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Absent | Self::Null => None,
        }
    }
}
