//! Concrete controlled metadata constructors join authentic component credits.
use super::NativeOwned;
use crate::model::{
    controls::*,
    extensions::Extensions,
    ids::*,
    json::deferred::{DecodeLedger, JsonDecodeError},
};
pub fn session_id(text: NativeOwned<String>) -> Result<NativeOwned<SessionId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = SessionId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn mode_id(text: NativeOwned<String>) -> Result<NativeOwned<ModeId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = ModeId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn config_id(text: NativeOwned<String>) -> Result<NativeOwned<ConfigId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = ConfigId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn config_choice_id(
    text: NativeOwned<String>,
) -> Result<NativeOwned<ConfigChoiceId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = ConfigChoiceId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn config_group_id(
    text: NativeOwned<String>,
) -> Result<NativeOwned<ConfigGroupId>, JsonDecodeError> {
    let ledger = text.ledger().share();
    let value = ConfigGroupId::admitted(text)?;
    Ok(NativeOwned::from_parts(value, ledger.reserve(0)?))
}
pub fn config_option(
    id: NativeOwned<ConfigId>,
    name: NativeOwned<String>,
    description: NativeOwned<Option<String>>,
    category: NativeOwned<Option<ConfigCategory>>,
    kind: NativeOwned<ConfigKind>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ConfigOption>, JsonDecodeError> {
    let ((((((id, name), description), category), kind), extensions), allocation) = id
        .join(name)?
        .join(description)?
        .join(category)?
        .join(kind)?
        .join(extensions)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        ConfigOption {
            id,
            name,
            description,
            category,
            kind,
            extensions,
        },
        allocation,
    ))
}
pub fn config_choice(
    id: NativeOwned<ConfigChoiceId>,
    name: NativeOwned<String>,
    description: NativeOwned<Option<String>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ConfigChoice>, JsonDecodeError> {
    let ((((id, name), description), extensions), allocation) = id
        .join(name)?
        .join(description)?
        .join(extensions)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        ConfigChoice {
            id,
            name,
            description,
            extensions,
        },
        allocation,
    ))
}
pub fn config_group(
    id: NativeOwned<ConfigGroupId>,
    name: NativeOwned<String>,
    choices: NativeOwned<Vec<ConfigChoice>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ConfigGroup>, JsonDecodeError> {
    let ((((id, name), choices), extensions), allocation) =
        id.join(name)?.join(choices)?.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        ConfigGroup {
            id,
            name,
            choices,
            extensions,
        },
        allocation,
    ))
}
pub fn mode_choice(
    id: NativeOwned<ModeId>,
    name: NativeOwned<String>,
    description: NativeOwned<Option<String>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<ModeChoice>, JsonDecodeError> {
    let ((((id, name), description), extensions), allocation) = id
        .join(name)?
        .join(description)?
        .join(extensions)?
        .into_parts();
    Ok(NativeOwned::from_parts(
        ModeChoice {
            id,
            name,
            description,
            extensions,
        },
        allocation,
    ))
}
pub fn modes(
    current: NativeOwned<ModeId>,
    choices: NativeOwned<Vec<ModeChoice>>,
    extensions: NativeOwned<Option<Extensions>>,
) -> Result<NativeOwned<Modes>, JsonDecodeError> {
    let (((current, choices), extensions), allocation) =
        current.join(choices)?.join(extensions)?.into_parts();
    Ok(NativeOwned::from_parts(
        Modes {
            current,
            choices,
            extensions,
        },
        allocation,
    ))
}
pub fn boolean(
    value: bool,
    ledger: &DecodeLedger,
) -> Result<NativeOwned<ConfigKind>, JsonDecodeError> {
    Ok(NativeOwned::from_parts(
        ConfigKind::Boolean(value),
        ledger.reserve(0)?,
    ))
}
pub fn select(
    current: NativeOwned<ConfigChoiceId>,
    choices: NativeOwned<SelectChoices>,
) -> Result<NativeOwned<ConfigKind>, JsonDecodeError> {
    let ((current, choices), allocation) = current.join(choices)?.into_parts();
    Ok(NativeOwned::from_parts(
        ConfigKind::Select { current, choices },
        allocation,
    ))
}
pub fn unsupported(label: NativeOwned<String>) -> NativeOwned<ConfigKind> {
    let (label, allocation) = label.into_parts();
    NativeOwned::from_parts(ConfigKind::Unsupported { label }, allocation)
}
pub fn flat(owned: NativeOwned<Vec<ConfigChoice>>) -> NativeOwned<SelectChoices> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SelectChoices::Flat(value), allocation)
}
pub fn groups(owned: NativeOwned<Vec<ConfigGroup>>) -> NativeOwned<SelectChoices> {
    let (value, allocation) = owned.into_parts();
    NativeOwned::from_parts(SelectChoices::Groups(value), allocation)
}
pub fn known(
    value: &'static str,
    ledger: &DecodeLedger,
) -> Result<Option<NativeOwned<ConfigCategory>>, JsonDecodeError> {
    let category = match value {
        "mode" => ConfigCategory::Mode,
        "model" => ConfigCategory::Model,
        "model_config" => ConfigCategory::ModelConfig,
        "thought_level" => ConfigCategory::ThoughtLevel,
        _ => return Ok(None),
    };
    Ok(Some(NativeOwned::from_parts(category, ledger.reserve(0)?)))
}
pub fn other(value: NativeOwned<String>) -> NativeOwned<ConfigCategory> {
    let (value, allocation) = value.into_parts();
    NativeOwned::from_parts(ConfigCategory::Other(value), allocation)
}
