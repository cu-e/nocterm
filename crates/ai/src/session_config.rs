//! Session configuration a chat can choose before its session opens.
//!
//! An agent reports configuration options (model, reasoning effort, …) only
//! for an open session. The options an agent last reported are kept in a
//! protocol-independent form, so a chat shows its pickers before it connects.
//! Choices made before the session opens, and the model and reasoning effort
//! last chosen for the agent, are applied as soon as it does.
use crate::model::{
    controls::{
        ConfigCategory, ConfigChoice, ConfigChoices, ConfigGroup, ConfigKind, ConfigOption,
        ConfigSelect, ConfigSetting,
    },
    ids::{ConfigChoiceId, ConfigId},
};
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
    pub fn from_setting(setting: &ConfigSetting) -> Self {
        match setting {
            ConfigSetting::Boolean(value) => Self::Flag(*value),
            ConfigSetting::Choice(value) => Self::Value(value.to_string()),
        }
    }
    pub fn to_setting(&self) -> ConfigSetting {
        match self {
            Self::Flag(value) => ConfigSetting::Boolean(*value),
            Self::Value(value) => ConfigSetting::Choice(ConfigChoiceId::new(value.clone())),
        }
    }
    fn current(option: &ConfigOption) -> Self {
        match &option.kind {
            ConfigKind::Select(select) => Self::Value(select.current.to_string()),
            ConfigKind::Boolean(current) => Self::Flag(*current),
        }
    }
}

/// Values chosen for a chat or an agent, by option id.
pub type Choices = BTreeMap<String, ConfigValue>;

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
        value: value.id.to_string(),
        name: value.name.clone(),
        description: value.description.clone(),
        group: group.map(|group| (group.id.to_string(), group.name.clone())),
    }
}

/// The options an agent reported, in storable form.
pub fn store(options: &[ConfigOption]) -> Vec<StoredOption> {
    options
        .iter()
        .map(|option| {
            let values = match &option.kind {
                ConfigKind::Select(select) => match &select.choices {
                    ConfigChoices::Flat(values) => values
                        .iter()
                        .map(|value| stored_value(value, None))
                        .collect(),
                    ConfigChoices::Grouped(groups) => groups
                        .iter()
                        .flat_map(|group| {
                            group
                                .choices
                                .iter()
                                .map(move |value| stored_value(value, Some(group)))
                        })
                        .collect(),
                },
                ConfigKind::Boolean(_) => Vec::new(),
            };
            StoredOption {
                id: option.id.to_string(),
                name: option.name.clone(),
                description: option.description.clone(),
                category: option.category.as_ref().map(category_name),
                current: ConfigValue::current(option),
                values,
            }
        })
        .collect()
}

/// Stored options as the agent would report them.
pub fn restore(stored: &[StoredOption]) -> Vec<ConfigOption> {
    stored
        .iter()
        .map(|option| {
            let kind = match &option.current {
                ConfigValue::Flag(value) => ConfigKind::Boolean(*value),
                ConfigValue::Value(current) => {
                    let value = |value: &StoredValue| ConfigChoice {
                        id: ConfigChoiceId::new(value.value.clone()),
                        name: value.name.clone(),
                        description: value.description.clone(),
                    };
                    let choices = if option.values.iter().any(|value| value.group.is_some()) {
                        let mut groups: Vec<ConfigGroup> = Vec::new();
                        for stored in &option.values {
                            let (id, name) = stored.group.clone().unwrap_or_default();
                            match groups.last_mut() {
                                Some(group) if group.id.as_str() == id => {
                                    group.choices.push(value(stored))
                                }
                                _ => groups.push(ConfigGroup::new(id, name, vec![value(stored)])),
                            }
                        }
                        ConfigChoices::Grouped(groups)
                    } else {
                        ConfigChoices::Flat(option.values.iter().map(value).collect())
                    };
                    ConfigKind::Select(ConfigSelect {
                        current: ConfigChoiceId::new(current.clone()),
                        choices,
                    })
                }
            };
            ConfigOption {
                id: ConfigId::new(option.id.clone()),
                name: option.name.clone(),
                description: option.description.clone(),
                category: option.category.as_deref().map(category),
                kind,
            }
        })
        .collect()
}

/// The values `options` currently have, by option id.
pub fn current(options: &[ConfigOption]) -> Choices {
    options
        .iter()
        .map(|option| (option.id.to_string(), ConfigValue::current(option)))
        .collect()
}

/// `options` showing the `choices` they offer as their current values.
pub fn with_choices(mut options: Vec<ConfigOption>, choices: &Choices) -> Vec<ConfigOption> {
    for (id, value) in choices {
        choose(&mut options, id, value);
    }
    options
}

/// Whether `option` offers `value`.
fn offers(option: &ConfigOption, value: &ConfigValue) -> bool {
    match (&option.kind, value) {
        (ConfigKind::Boolean(_), ConfigValue::Flag(_)) => true,
        (ConfigKind::Select(select), ConfigValue::Value(value)) => match &select.choices {
            ConfigChoices::Flat(values) => {
                values.iter().any(|offered| offered.id.as_str() == value)
            }
            ConfigChoices::Grouped(groups) => groups
                .iter()
                .flat_map(|group| &group.choices)
                .any(|offered| offered.id.as_str() == value),
        },
        _ => false,
    }
}

/// Shows `value` as the current value of option `id` until the agent
/// reports otherwise. Returns whether the option offers it.
pub fn choose(options: &mut [ConfigOption], id: &str, value: &ConfigValue) -> bool {
    let Some(option) = options
        .iter_mut()
        .find(|option| option.id.as_str() == id && offers(option, value))
    else {
        return false;
    };
    match (&mut option.kind, value) {
        (ConfigKind::Boolean(current), ConfigValue::Flag(value)) => *current = *value,
        (ConfigKind::Select(select), ConfigValue::Value(value)) => {
            select.current = ConfigChoiceId::new(value.clone())
        }
        _ => return false,
    }
    true
}

/// The requests that give an open session the `choices`, in the agent's
/// option order: a model is set before the reasoning effort it offers.
/// Choices the session already has, or no longer offers, are skipped.
pub fn requests(options: &[ConfigOption], choices: &Choices) -> Vec<(ConfigId, ConfigValue)> {
    options
        .iter()
        .filter_map(|option| {
            let value = choices.get(option.id.as_str())?;
            (offers(option, value) && ConfigValue::current(option) != *value)
                .then(|| (option.id.clone(), value.clone()))
        })
        .collect()
}

/// Whether a choice of `option` carries over to the agent's next chats. The
/// mode is not: a permissive mode must be chosen for each chat.
pub fn remembered(option: &ConfigOption) -> bool {
    matches!(
        option.category,
        Some(ConfigCategory::Model | ConfigCategory::ThoughtLevel)
    )
}

#[cfg(test)]
mod tests;
