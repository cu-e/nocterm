use super::content::extensions;
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::retention::RetentionAuthority;
use nocterm_ai::model::{controls::*, diagnostic::ModelError, ids::*};
fn choice(
    authority: &RetentionAuthority,
    value: wire::SessionConfigSelectOption,
) -> Result<ConfigChoice, ModelError> {
    Ok(ConfigChoice {
        id: ConfigChoiceId::new(value.value.0.as_ref().to_owned()),
        name: value.name,
        description: value.description,
        extensions: extensions(authority, value.meta)?,
    })
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
pub(crate) fn option(
    authority: &RetentionAuthority,
    value: wire::SessionConfigOption,
) -> Result<ConfigOption, ModelError> {
    let kind = match value.kind {
        wire::SessionConfigKind::Boolean(value) => ConfigKind::Boolean(value.current_value),
        wire::SessionConfigKind::Select(value) => ConfigKind::Select {
            current: ConfigChoiceId::new(value.current_value.0.as_ref().to_owned()),
            choices: match value.options {
                wire::SessionConfigSelectOptions::Ungrouped(values) => SelectChoices::Flat(
                    values
                        .into_iter()
                        .map(|value| choice(authority, value))
                        .collect::<Result<_, _>>()?,
                ),
                wire::SessionConfigSelectOptions::Grouped(groups) => SelectChoices::Groups(
                    groups
                        .into_iter()
                        .map(|group| {
                            Ok(ConfigGroup {
                                id: ConfigGroupId::new(group.group.0.as_ref().to_owned()),
                                name: group.name,
                                choices: group
                                    .options
                                    .into_iter()
                                    .map(|value| choice(authority, value))
                                    .collect::<Result<_, _>>()?,
                                extensions: extensions(authority, group.meta)?,
                            })
                        })
                        .collect::<Result<_, ModelError>>()?,
                ),
                _ => return Err(ModelError::Unsupported("configuration choices")),
            },
        },
        _ => return Err(ModelError::Unsupported("configuration kind")),
    };
    Ok(ConfigOption {
        id: ConfigId::new(value.id.0.as_ref().to_owned()),
        name: value.name,
        description: value.description,
        category: value.category.map(category).transpose()?,
        kind,
        extensions: extensions(authority, value.meta)?,
    })
}
