use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{
    controls::*,
    diagnostic::ModelError,
    ids::{ConfigChoiceId, ConfigGroupId, ConfigId, ModeId},
};

fn choice(value: wire::SessionConfigSelectOption) -> ConfigChoice {
    ConfigChoice {
        id: ConfigChoiceId::new(value.value.to_string()),
        name: value.name,
        description: value.description,
    }
}
fn category(value: wire::SessionConfigOptionCategory) -> Result<ConfigCategory, ModelError> {
    Ok(match value {
        wire::SessionConfigOptionCategory::Mode => ConfigCategory::Mode,
        wire::SessionConfigOptionCategory::Model => ConfigCategory::Model,
        wire::SessionConfigOptionCategory::ModelConfig => ConfigCategory::ModelConfig,
        wire::SessionConfigOptionCategory::ThoughtLevel => ConfigCategory::ThoughtLevel,
        wire::SessionConfigOptionCategory::Other(value) => ConfigCategory::Other(value),
        _ => return Err(ModelError::Unsupported("configuration category")),
    })
}
pub(super) fn option(value: wire::SessionConfigOption) -> Result<ConfigOption, ModelError> {
    let kind = match value.kind {
        wire::SessionConfigKind::Boolean(value) => ConfigKind::Boolean(value.current_value),
        wire::SessionConfigKind::Select(value) => ConfigKind::Select(ConfigSelect {
            current: ConfigChoiceId::new(value.current_value.to_string()),
            choices: match value.options {
                wire::SessionConfigSelectOptions::Ungrouped(values) => {
                    ConfigChoices::Flat(values.into_iter().map(choice).collect())
                }
                wire::SessionConfigSelectOptions::Grouped(groups) => ConfigChoices::Grouped(
                    groups
                        .into_iter()
                        .map(|group| ConfigGroup {
                            id: ConfigGroupId::new(group.group.to_string()),
                            name: group.name,
                            choices: group.options.into_iter().map(choice).collect(),
                        })
                        .collect(),
                ),
                _ => return Err(ModelError::Unsupported("configuration choices")),
            },
        }),
        _ => return Err(ModelError::Unsupported("configuration kind")),
    };
    Ok(ConfigOption {
        id: ConfigId::new(value.id.to_string()),
        name: value.name,
        description: value.description,
        category: value.category.map(category).transpose()?,
        kind,
    })
}
pub(super) fn options(
    values: Vec<wire::SessionConfigOption>,
) -> Result<Vec<ConfigOption>, ModelError> {
    values.into_iter().map(option).collect()
}
pub(super) fn modes(value: wire::SessionModeState) -> Modes {
    Modes {
        current: ModeId::new(value.current_mode_id.to_string()),
        choices: value
            .available_modes
            .into_iter()
            .map(|mode| ModeChoice {
                id: ModeId::new(mode.id.to_string()),
                name: mode.name,
                description: mode.description,
            })
            .collect(),
    }
}
pub(super) fn wire_setting(value: ConfigSetting) -> wire::SessionConfigOptionValue {
    match value {
        ConfigSetting::Boolean(value) => wire::SessionConfigOptionValue::boolean(value),
        ConfigSetting::Choice(id) => wire::SessionConfigOptionValue::value_id(id.to_string()),
    }
}
