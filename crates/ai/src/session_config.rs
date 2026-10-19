//! Session configuration a chat can choose before its session opens.
//!
//! An agent reports configuration options (model, reasoning effort, …) only
//! for an open session. The options an agent last reported are kept in a
//! protocol-independent form, so a chat shows its pickers before it connects.
//! Choices made before the session opens, and the model and reasoning effort
//! last chosen for the agent, are applied as soon as it does.
use crate::model::{controls::*, ids::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The value of one option, as stored and as chosen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    Flag(bool),
    Value(String),
}
impl ConfigValue {
    fn current(option: &ConfigOption) -> Option<Self> {
        match &option.kind {
            ConfigKind::Select { current, .. } => Some(Self::Value(current.as_str().into())),
            ConfigKind::Boolean(value) => Some(Self::Flag(*value)),
            ConfigKind::Unsupported { .. } => None,
        }
    }
}

/// Values chosen for a chat or an agent, by option id.
pub type Choices = BTreeMap<String, ConfigValue>;

/// Backing envelope for the pinned Rust 1.98 B-tree implementation (B=6).
/// An internal node contains at most eleven keys/values, twelve edges and
/// parent/index/length fields. Sixteen pointer widths cover edges, header and
/// alignment padding on supported targets. Payload String buffers are separate.
fn choice_node_bound() -> Option<usize> {
    use std::mem::size_of;
    size_of::<String>()
        .checked_add(size_of::<ConfigValue>())?
        .checked_mul(11)?
        .checked_add(size_of::<usize>().checked_mul(16)?)
}
/// Stable nodes are nonempty, so their count never exceeds the number of keys.
pub fn choices_retained_backing_bound(entries: usize) -> Option<usize> {
    choice_node_bound()?.checked_mul(entries)
}
/// Reserve before insertion; two extra nodes cover an unfinished split and root
/// promotion while the old nodes and all text payloads remain alive.
pub fn choices_construction_backing_bound(entries: usize) -> Option<usize> {
    choice_node_bound()?.checked_mul(entries.checked_add(2)?)
}

/// One option an agent reported, independent of the protocol schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredOption {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub current: ConfigValue,
    /// The choices of a select; empty for a flag.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<StoredValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredValue {
    pub value: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The id and name of the group the value is listed under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<(String, String)>,
}

fn category_name(category: &ConfigCategory) -> String {
    match category {
        ConfigCategory::Mode => "mode".into(),
        ConfigCategory::Model => "model".into(),
        ConfigCategory::ModelConfig => "model_config".into(),
        ConfigCategory::ThoughtLevel => "thought_level".into(),
        ConfigCategory::Other(name) => name.clone(),
    }
}
fn category(name: &str) -> ConfigCategory {
    match name {
        "mode" => ConfigCategory::Mode,
        "model" => ConfigCategory::Model,
        "model_config" => ConfigCategory::ModelConfig,
        "thought_level" => ConfigCategory::ThoughtLevel,
        other => ConfigCategory::Other(other.into()),
    }
}
fn stored_value(value: &ConfigChoice, group: Option<&ConfigGroup>) -> StoredValue {
    StoredValue {
        value: value.id.as_str().into(),
        name: value.name.clone(),
        description: value.description.clone(),
        group: group.map(|group| (group.id.as_str().into(), group.name.clone())),
    }
}
/// The options an agent reported, in the unchanged preference grammar.
pub fn store(options: &[ConfigOption]) -> Vec<StoredOption> {
    options
        .iter()
        .filter_map(|option| {
            let values = match &option.kind {
                ConfigKind::Select {
                    choices: SelectChoices::Flat(values),
                    ..
                } => values
                    .iter()
                    .map(|value| stored_value(value, None))
                    .collect(),
                ConfigKind::Select {
                    choices: SelectChoices::Groups(groups),
                    ..
                } => groups
                    .iter()
                    .flat_map(|group| {
                        group
                            .choices
                            .iter()
                            .map(move |value| stored_value(value, Some(group)))
                    })
                    .collect(),
                _ => Vec::new(),
            };
            Some(StoredOption {
                id: option.id.as_str().into(),
                name: option.name.clone(),
                description: option.description.clone(),
                category: option.category.as_ref().map(category_name),
                current: ConfigValue::current(option)?,
                values,
            })
        })
        .collect()
}
/// Restore presentation options without native or provider lookup.
pub fn restore(stored: &[StoredOption]) -> Vec<ConfigOption> {
    stored
        .iter()
        .map(|option| {
            let kind = match &option.current {
                ConfigValue::Flag(value) => ConfigKind::Boolean(*value),
                ConfigValue::Value(current) => {
                    let choice = |value: &StoredValue| ConfigChoice {
                        id: ConfigChoiceId::new(value.value.clone()),
                        name: value.name.clone(),
                        description: value.description.clone(),
                        extensions: None,
                    };
                    let choices = if option.values.iter().any(|value| value.group.is_some()) {
                        let mut groups: Vec<ConfigGroup> = Vec::new();
                        for stored in &option.values {
                            let (id, name) = stored.group.clone().unwrap_or_default();
                            match groups.last_mut() {
                                Some(group) if group.id.as_str() == id => {
                                    group.choices.push(choice(stored))
                                }
                                _ => groups.push(ConfigGroup {
                                    id: ConfigGroupId::new(id),
                                    name,
                                    choices: vec![choice(stored)],
                                    extensions: None,
                                }),
                            }
                        }
                        SelectChoices::Groups(groups)
                    } else {
                        SelectChoices::Flat(option.values.iter().map(choice).collect())
                    };
                    ConfigKind::Select {
                        current: ConfigChoiceId::new(current.clone()),
                        choices,
                    }
                }
            };
            ConfigOption {
                id: ConfigId::new(option.id.clone()),
                name: option.name.clone(),
                description: option.description.clone(),
                category: option.category.as_deref().map(category),
                kind,
                extensions: None,
            }
        })
        .collect()
}
pub fn current(options: &[ConfigOption]) -> Choices {
    options
        .iter()
        .filter_map(|option| Some((option.id.as_str().into(), ConfigValue::current(option)?)))
        .collect()
}
pub fn with_choices(mut options: Vec<ConfigOption>, choices: &Choices) -> Vec<ConfigOption> {
    for (id, value) in choices {
        choose(&mut options, id, value);
    }
    options
}
fn offers(option: &ConfigOption, value: &ConfigValue) -> bool {
    match (&option.kind, value) {
        (ConfigKind::Boolean(_), ConfigValue::Flag(_)) => true,
        (
            ConfigKind::Select {
                choices: SelectChoices::Flat(choices),
                ..
            },
            ConfigValue::Value(value),
        ) => choices.iter().any(|choice| choice.id.as_str() == value),
        (
            ConfigKind::Select {
                choices: SelectChoices::Groups(groups),
                ..
            },
            ConfigValue::Value(value),
        ) => groups
            .iter()
            .flat_map(|group| &group.choices)
            .any(|choice| choice.id.as_str() == value),
        _ => false,
    }
}
pub fn choose(options: &mut [ConfigOption], id: &str, value: &ConfigValue) -> bool {
    let Some(option) = options
        .iter_mut()
        .find(|option| option.id.as_str() == id && offers(option, value))
    else {
        return false;
    };
    match (&mut option.kind, value) {
        (ConfigKind::Boolean(current), ConfigValue::Flag(value)) => *current = *value,
        (ConfigKind::Select { current, .. }, ConfigValue::Value(value)) => {
            *current = ConfigChoiceId::new(value.clone())
        }
        _ => return false,
    }
    true
}
/// Preserve agent option order and skip unchanged/unoffered selections.
pub fn requests(options: &[ConfigOption], choices: &Choices) -> Vec<(ConfigId, ConfigValue)> {
    options
        .iter()
        .filter_map(|option| {
            let value = choices.get(option.id.as_str())?;
            (offers(option, value) && ConfigValue::current(option).as_ref() != Some(value))
                .then(|| (option.id.clone(), value.clone()))
        })
        .collect()
}
/// Permissive modes never carry over to another chat.
pub fn remembered(option: &ConfigOption) -> bool {
    matches!(
        option.category,
        Some(ConfigCategory::Model | ConfigCategory::ThoughtLevel)
    )
}
#[cfg(test)]
mod tests;
