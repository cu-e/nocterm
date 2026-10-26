//! The `[settings]` table of a package, mapped onto the shared settings schema.
//!
//! The kinds are `toggle`, `choice`, `multi-choice`, `number` and `text`; the limits and the
//! validation are the schema's. Labels, help and option labels are sanitized, at most 48 and 160
//! characters.

use alloc::{borrow::Cow, string::String, vec::Vec};

use nocterm_plugin_model::{
    ChoiceOption, ContributionId, FieldGroup, FieldKind, FieldSpec, PluginId, SettingsSchema,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::sanitize_display;

const MAX_LABEL: usize = 48;
const MAX_HELP: usize = 160;

/// The kinds of setting an author writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FieldKindName {
    Toggle,
    Choice,
    MultiChoice,
    Number,
    Text,
}

/// The default of a field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DefaultDto {
    Bool(bool),
    Number(i32),
    Text(String),
    List(Vec<String>),
}

/// An option of a choice or multi-choice field.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ChoiceOptionDto {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub help: Option<String>,
}

/// One setting as written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct FieldDto {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub help: Option<String>,
    pub kind: Option<FieldKindName>,
    #[serde(default)]
    pub renamed_from: Option<String>,
    #[serde(default)]
    pub default: Option<DefaultDto>,
    /// For `choice` and `multi-choice`.
    #[serde(default)]
    pub options: Vec<ChoiceOptionDto>,
    /// For `multi-choice`: whether the order of the chosen options matters.
    #[serde(default)]
    pub ordered: Option<bool>,
    /// For `multi-choice`: the most options that may be chosen.
    #[serde(default)]
    pub max_chosen: Option<u8>,
    /// For `number`.
    #[serde(default)]
    pub min: Option<i32>,
    #[serde(default)]
    pub max: Option<i32>,
    #[serde(default)]
    pub step: Option<i32>,
    #[serde(default)]
    pub unit: Option<String>,
    /// For `text`, in characters.
    #[serde(default)]
    pub max_len: Option<u16>,
    #[serde(default)]
    pub placeholder: Option<String>,
}

/// A titled run of settings, for the plugin or for one of its contributions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct GroupDto {
    pub title: String,
    /// The local name of the contribution the group is shown for; absent for the plugin.
    #[serde(default)]
    pub contribution: Option<String>,
    #[serde(default)]
    pub field: Vec<FieldDto>,
}

/// The `[settings]` table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SettingsDto {
    /// The revision of the schema, raised when fields change. Default 1.
    #[serde(default)]
    pub version: Option<u16>,
    #[serde(default)]
    pub group: Vec<GroupDto>,
}

/// A setting the host refuses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SettingsError {
    #[error("setting `{key}` needs a `kind`")]
    NoKind { key: String },
    #[error("setting `{key}` does not take `{field}` for this kind")]
    Stray { key: String, field: &'static str },
    #[error("setting `{key}` needs `{field}`")]
    Missing { key: String, field: &'static str },
    #[error("setting `{key}` has a default of the wrong type")]
    Default { key: String },
    #[error("the group `{title}` names a contribution that is not a valid local name")]
    Group { title: String },
}

pub(super) fn schema(
    plugin: &PluginId,
    dto: Option<&SettingsDto>,
) -> Result<SettingsSchema, SettingsError> {
    let Some(dto) = dto else {
        return Ok(SettingsSchema::NONE);
    };
    let mut groups = Vec::with_capacity(dto.group.len());
    for group in &dto.group {
        let contribution = group
            .contribution
            .as_deref()
            .map(|local| ContributionId::in_plugin(plugin, local))
            .transpose()
            .map_err(|_| SettingsError::Group {
                title: group.title.chars().take(MAX_LABEL).collect(),
            })?;
        let fields = group
            .field
            .iter()
            .map(field)
            .collect::<Result<Vec<_>, _>>()?;
        groups.push(FieldGroup {
            title: Cow::Owned(sanitize_display(&group.title, MAX_LABEL)),
            contribution,
            fields: Cow::Owned(fields),
        });
    }
    Ok(SettingsSchema {
        version: dto.version.unwrap_or(1),
        groups: Cow::Owned(groups),
    })
}

fn field(dto: &FieldDto) -> Result<FieldSpec, SettingsError> {
    let key = dto.key.clone();
    let kind_name = dto
        .kind
        .ok_or_else(|| SettingsError::NoKind { key: key.clone() })?;
    stray(dto, kind_name)?;
    let kind = match kind_name {
        FieldKindName::Toggle => FieldKind::Toggle {
            default: match &dto.default {
                None => false,
                Some(DefaultDto::Bool(value)) => *value,
                Some(_) => return Err(SettingsError::Default { key }),
            },
        },
        FieldKindName::Choice => FieldKind::Choice {
            options: options(dto),
            default: match &dto.default {
                Some(DefaultDto::Text(value)) => Cow::Owned(value.clone()),
                _ => return Err(SettingsError::Default { key }),
            },
        },
        FieldKindName::MultiChoice => FieldKind::MultiChoice {
            options: options(dto),
            default: match &dto.default {
                None => Cow::Owned(Vec::new()),
                Some(DefaultDto::List(values)) => {
                    Cow::Owned(values.iter().cloned().map(Cow::Owned).collect())
                }
                Some(_) => return Err(SettingsError::Default { key }),
            },
            ordered: dto.ordered.unwrap_or(false),
            max: dto.max_chosen.ok_or(SettingsError::Missing {
                key,
                field: "max_chosen",
            })?,
        },
        FieldKindName::Number => number(dto)?,
        FieldKindName::Text => FieldKind::Text {
            default: match &dto.default {
                None => Cow::Borrowed(""),
                Some(DefaultDto::Text(value)) => Cow::Owned(value.clone()),
                Some(_) => return Err(SettingsError::Default { key }),
            },
            max_len: dto.max_len.ok_or(SettingsError::Missing {
                key,
                field: "max_len",
            })?,
            placeholder: dto
                .placeholder
                .as_deref()
                .map(|text| Cow::Owned(sanitize_display(text, MAX_HELP))),
        },
    };
    Ok(FieldSpec {
        key: Cow::Owned(dto.key.clone()),
        label: Cow::Owned(sanitize_display(&dto.label, MAX_LABEL)),
        help: dto
            .help
            .as_deref()
            .map(|text| Cow::Owned(sanitize_display(text, MAX_HELP))),
        kind,
        renamed_from: dto.renamed_from.clone().map(Cow::Owned),
    })
}

fn number(dto: &FieldDto) -> Result<FieldKind, SettingsError> {
    let missing = |field| SettingsError::Missing {
        key: dto.key.clone(),
        field,
    };
    let min = dto.min.ok_or_else(|| missing("min"))?;
    let default = match &dto.default {
        None => min,
        Some(DefaultDto::Number(value)) => *value,
        Some(_) => {
            return Err(SettingsError::Default {
                key: dto.key.clone(),
            });
        }
    };
    Ok(FieldKind::Number {
        min,
        max: dto.max.ok_or_else(|| missing("max"))?,
        step: dto.step.unwrap_or(1),
        default,
        unit: dto
            .unit
            .as_deref()
            .map(|text| Cow::Owned(sanitize_display(text, 8))),
    })
}

fn options(dto: &FieldDto) -> Cow<'static, [ChoiceOption]> {
    Cow::Owned(
        dto.options
            .iter()
            .map(|option| ChoiceOption {
                key: Cow::Owned(option.key.clone()),
                label: Cow::Owned(sanitize_display(&option.label, MAX_LABEL)),
                help: option
                    .help
                    .as_deref()
                    .map(|text| Cow::Owned(sanitize_display(text, MAX_HELP))),
            })
            .collect(),
    )
}

/// Refuses a key that belongs to another kind, so a typo is an error and not a silent no-op.
fn stray(dto: &FieldDto, kind: FieldKindName) -> Result<(), SettingsError> {
    use FieldKindName::{Choice, MultiChoice, Number, Text};
    let choosing = matches!(kind, Choice | MultiChoice);
    let found = [
        ("options", !dto.options.is_empty() && !choosing),
        ("ordered", dto.ordered.is_some() && kind != MultiChoice),
        (
            "max_chosen",
            dto.max_chosen.is_some() && kind != MultiChoice,
        ),
        ("min", dto.min.is_some() && kind != Number),
        ("max", dto.max.is_some() && kind != Number),
        ("step", dto.step.is_some() && kind != Number),
        ("unit", dto.unit.is_some() && kind != Number),
        ("max_len", dto.max_len.is_some() && kind != Text),
        ("placeholder", dto.placeholder.is_some() && kind != Text),
    ];
    match found.into_iter().find(|(_, stray)| *stray) {
        Some((field, _)) => Err(SettingsError::Stray {
            key: dto.key.clone(),
            field,
        }),
        None => Ok(()),
    }
}
