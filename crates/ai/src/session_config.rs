//! Session configuration a chat can choose before its session opens.
//!
//! An agent reports configuration options (model, reasoning effort, …) only
//! for an open session. The options an agent last reported are kept in a
//! protocol-independent form, so a chat shows its pickers before it connects.
//! Choices made before the session opens, and the model and reasoning effort
//! last chosen for the agent, are applied as soon as it does.
use crate::acp;
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
    pub fn from_acp(value: &acp::SessionConfigOptionValue) -> Option<Self> {
        match value {
            acp::SessionConfigOptionValue::Boolean { value } => Some(Self::Flag(*value)),
            acp::SessionConfigOptionValue::ValueId { value } => {
                Some(Self::Value(value.to_string()))
            }
            _ => None,
        }
    }
    pub fn to_acp(&self) -> acp::SessionConfigOptionValue {
        match self {
            Self::Flag(value) => acp::SessionConfigOptionValue::boolean(*value),
            Self::Value(value) => acp::SessionConfigOptionValue::value_id(value.clone()),
        }
    }
    fn current(option: &acp::SessionConfigOption) -> Option<Self> {
        match &option.kind {
            acp::SessionConfigKind::Select(select) => {
                Some(Self::Value(select.current_value.to_string()))
            }
            acp::SessionConfigKind::Boolean(boolean) => Some(Self::Flag(boolean.current_value)),
            _ => None,
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

fn category_name(category: &acp::SessionConfigOptionCategory) -> String {
    match category {
        acp::SessionConfigOptionCategory::Mode => "mode".into(),
        acp::SessionConfigOptionCategory::Model => "model".into(),
        acp::SessionConfigOptionCategory::ModelConfig => "model_config".into(),
        acp::SessionConfigOptionCategory::ThoughtLevel => "thought_level".into(),
        acp::SessionConfigOptionCategory::Other(name) => name.clone(),
        _ => "other".into(),
    }
}
fn category(name: &str) -> acp::SessionConfigOptionCategory {
    match name {
        "mode" => acp::SessionConfigOptionCategory::Mode,
        "model" => acp::SessionConfigOptionCategory::Model,
        "model_config" => acp::SessionConfigOptionCategory::ModelConfig,
        "thought_level" => acp::SessionConfigOptionCategory::ThoughtLevel,
        other => acp::SessionConfigOptionCategory::Other(other.into()),
    }
}

fn stored_value(
    value: &acp::SessionConfigSelectOption,
    group: Option<&acp::SessionConfigSelectGroup>,
) -> StoredValue {
    StoredValue {
        value: value.value.to_string(),
        name: value.name.clone(),
        description: value.description.clone(),
        group: group.map(|group| (group.group.to_string(), group.name.clone())),
    }
}

/// The options an agent reported, in storable form.
pub fn store(options: &[acp::SessionConfigOption]) -> Vec<StoredOption> {
    options
        .iter()
        .filter_map(|option| {
            let values = match &option.kind {
                acp::SessionConfigKind::Select(select) => match &select.options {
                    acp::SessionConfigSelectOptions::Ungrouped(values) => values
                        .iter()
                        .map(|value| stored_value(value, None))
                        .collect(),
                    acp::SessionConfigSelectOptions::Grouped(groups) => groups
                        .iter()
                        .flat_map(|group| {
                            group
                                .options
                                .iter()
                                .map(move |value| stored_value(value, Some(group)))
                        })
                        .collect(),
                    _ => return None,
                },
                _ => Vec::new(),
            };
            Some(StoredOption {
                id: option.id.to_string(),
                name: option.name.clone(),
                description: option.description.clone(),
                category: option.category.as_ref().map(category_name),
                current: ConfigValue::current(option)?,
                values,
            })
        })
        .collect()
}

/// Stored options as the agent would report them.
pub fn restore(stored: &[StoredOption]) -> Vec<acp::SessionConfigOption> {
    stored
        .iter()
        .map(|option| {
            let kind = match &option.current {
                ConfigValue::Flag(value) => {
                    acp::SessionConfigKind::Boolean(acp::SessionConfigBoolean::new(*value))
                }
                ConfigValue::Value(current) => {
                    let value = |value: &StoredValue| {
                        acp::SessionConfigSelectOption::new(value.value.clone(), value.name.clone())
                            .description(value.description.clone())
                    };
                    let options = if option.values.iter().any(|value| value.group.is_some()) {
                        let mut groups: Vec<acp::SessionConfigSelectGroup> = Vec::new();
                        for stored in &option.values {
                            let (id, name) = stored.group.clone().unwrap_or_default();
                            match groups.last_mut() {
                                Some(group) if group.group.to_string() == id => {
                                    group.options.push(value(stored))
                                }
                                _ => groups.push(acp::SessionConfigSelectGroup::new(
                                    id,
                                    name,
                                    vec![value(stored)],
                                )),
                            }
                        }
                        acp::SessionConfigSelectOptions::Grouped(groups)
                    } else {
                        acp::SessionConfigSelectOptions::Ungrouped(
                            option.values.iter().map(value).collect(),
                        )
                    };
                    acp::SessionConfigKind::Select(acp::SessionConfigSelect::new(
                        current.clone(),
                        options,
                    ))
                }
            };
            acp::SessionConfigOption::new(option.id.clone(), option.name.clone(), kind)
                .description(option.description.clone())
                .category(option.category.as_deref().map(category))
        })
        .collect()
}

/// Whether `option` offers `value`.
fn offers(option: &acp::SessionConfigOption, value: &ConfigValue) -> bool {
    match (&option.kind, value) {
        (acp::SessionConfigKind::Boolean(_), ConfigValue::Flag(_)) => true,
        (acp::SessionConfigKind::Select(select), ConfigValue::Value(value)) => {
            match &select.options {
                acp::SessionConfigSelectOptions::Ungrouped(values) => {
                    values.iter().any(|offered| *offered.value.0 == **value)
                }
                acp::SessionConfigSelectOptions::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| &group.options)
                    .any(|offered| *offered.value.0 == **value),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Shows `value` as the current value of option `id` until the agent
/// reports otherwise. Returns whether the option offers it.
pub fn choose(options: &mut [acp::SessionConfigOption], id: &str, value: &ConfigValue) -> bool {
    let Some(option) = options
        .iter_mut()
        .find(|option| *option.id.0 == *id && offers(option, value))
    else {
        return false;
    };
    match (&mut option.kind, value) {
        (acp::SessionConfigKind::Boolean(boolean), ConfigValue::Flag(value)) => {
            boolean.current_value = *value
        }
        (acp::SessionConfigKind::Select(select), ConfigValue::Value(value)) => {
            select.current_value = value.clone().into()
        }
        _ => return false,
    }
    true
}

/// The requests that give an open session the `choices`, in the agent's
/// option order: a model is set before the reasoning effort it offers.
/// Choices the session already has, or no longer offers, are skipped.
pub fn requests(
    options: &[acp::SessionConfigOption],
    choices: &Choices,
) -> Vec<(acp::SessionConfigId, ConfigValue)> {
    options
        .iter()
        .filter_map(|option| {
            let value = choices.get(&*option.id.0)?;
            (offers(option, value) && ConfigValue::current(option).as_ref() != Some(value))
                .then(|| (option.id.clone(), value.clone()))
        })
        .collect()
}

/// Whether a choice of `option` carries over to the agent's next chats. The
/// mode is not: a permissive mode must be chosen for each chat.
pub fn remembered(option: &acp::SessionConfigOption) -> bool {
    matches!(
        option.category,
        Some(
            acp::SessionConfigOptionCategory::Model
                | acp::SessionConfigOptionCategory::ThoughtLevel
        )
    )
}

#[cfg(test)]
mod tests;
