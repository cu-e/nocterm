//! Controlled configuration and mode lists preserve SDK untagged/default policy.
use super::{content, error::DecodeError, lists, value::Value};
use nocterm_ai::model::{
    controls::*,
    json::deferred::JsonDecodeError,
    native::{NativeList, NativeOwned},
};
type Error = DecodeError<JsonDecodeError>;
pub(super) fn options(value: Value<'_>) -> Result<NativeOwned<Vec<ConfigOption>>, Error> {
    lists::skipped(
        value,
        |value| Ok(lists::shape(option_shape(value.clone()))?.is_some()),
        option,
    )
}
fn required_text(value: Option<Value<'_>>) -> Result<Value<'_>, Error> {
    let value = Value::required(value, "configuration text")?;
    if !value.raw().starts_with('"') {
        return Err(DecodeError::Field("configuration text"));
    }
    Ok(value)
}
fn choice_shape(value: Value<'_>) -> Result<(), Error> {
    let [id, name, _description, _meta] =
        value.fields(["value", "name", "description", "_meta"])?;
    required_text(id)?;
    required_text(name)?;
    Ok(())
}
fn group_shape(value: Value<'_>) -> Result<(), Error> {
    let [id, name, options, _meta] = value.fields(["group", "name", "options", "_meta"])?;
    required_text(id)?;
    required_text(name)?;
    Value::required(options, "group.options")?;
    Ok(())
}
/// The untagged schema accepts a wholly valid flat list first, then a wholly
/// valid group list. It does not individually skip mixed flat/group members.
fn choices_shape(value: Value<'_>) -> Result<bool, Error> {
    let mut elements = value.elements()?;
    let mut flat = true;
    let mut grouped = true;
    while let Some(item) = elements.next()? {
        flat &= lists::shape(choice_shape(item.clone()))?.is_some();
        grouped &= lists::shape(group_shape(item))?.is_some();
    }
    if flat {
        Ok(true)
    } else if grouped {
        Ok(false)
    } else {
        Err(DecodeError::Field("configuration choices"))
    }
}
fn option_shape(value: Value<'_>) -> Result<(), Error> {
    if !value.raw().starts_with('{') {
        return Err(DecodeError::Field("configuration object"));
    }
    let [id, name, kind, current, options] =
        value.fields(["id", "name", "type", "currentValue", "options"])?;
    required_text(id)?;
    required_text(name)?;
    let kind = required_text(kind)?;
    if kind.equals("boolean")? {
        let current = Value::required(current, "currentValue")?;
        if !matches!(current.raw(), "true" | "false") {
            return Err(DecodeError::Field("currentValue"));
        }
    } else if kind.equals("select")? {
        required_text(current)?;
        choices_shape(Value::required(options, "options")?)?;
    } else {
        return Err(DecodeError::Field("configuration kind"));
    }
    Ok(())
}
fn option(value: Value<'_>) -> Result<NativeOwned<ConfigOption>, Error> {
    let [
        id,
        name,
        description,
        category,
        kind,
        current,
        options,
        meta,
    ] = value.fields([
        "id",
        "name",
        "description",
        "category",
        "type",
        "currentValue",
        "options",
        "_meta",
    ])?;
    let kind = required_text(kind)?;
    let kind = if kind.equals("boolean")? {
        nocterm_ai::model::native::controls::boolean(
            Value::required(current, "currentValue")?.raw() == "true",
            value.ledger(),
        )?
    } else {
        nocterm_ai::model::native::controls::select(
            nocterm_ai::model::native::controls::config_choice_id(required_text(current)?.text()?)?,
            choices(Value::required(options, "options")?)?,
        )?
    };
    let category = match category.filter(|value| value.raw().starts_with('"')) {
        Some(category) => {
            let mut known = None;
            for key in ["mode", "model", "model_config", "thought_level"] {
                if category.equals(key)? {
                    known = nocterm_ai::model::native::controls::known(key, value.ledger())?;
                    break;
                }
            }
            NativeOwned::supplied(match known {
                Some(known) => known,
                None => nocterm_ai::model::native::controls::other(category.text()?),
            })
        }
        None => NativeOwned::<ConfigCategory>::absent(value.ledger())?,
    };
    Ok(nocterm_ai::model::native::controls::config_option(
        nocterm_ai::model::native::controls::config_id(required_text(id)?.text()?)?,
        required_text(name)?.text()?,
        content::optional_text(description, &value)?,
        category,
        kind,
        content::extensions(meta, &value)?,
    )?)
}
fn choices(value: Value<'_>) -> Result<NativeOwned<SelectChoices>, Error> {
    if choices_shape(value.clone())? {
        let mut result = NativeList::new(value.ledger(), value.array_len()?)?;
        let mut items = value.elements()?;
        while let Some(item) = items.next()? {
            result.push(choice(item)?)?;
        }
        Ok(nocterm_ai::model::native::controls::flat(result.finish()))
    } else {
        let mut result = NativeList::new(value.ledger(), value.array_len()?)?;
        let mut items = value.elements()?;
        while let Some(item) = items.next()? {
            result.push(group(item)?)?;
        }
        Ok(nocterm_ai::model::native::controls::groups(result.finish()))
    }
}
fn choice(value: Value<'_>) -> Result<NativeOwned<ConfigChoice>, Error> {
    let [id, name, description, meta] = value.fields(["value", "name", "description", "_meta"])?;
    Ok(nocterm_ai::model::native::controls::config_choice(
        nocterm_ai::model::native::controls::config_choice_id(required_text(id)?.text()?)?,
        required_text(name)?.text()?,
        content::optional_text(description, &value)?,
        content::extensions(meta, &value)?,
    )?)
}
fn group(value: Value<'_>) -> Result<NativeOwned<ConfigGroup>, Error> {
    let [id, name, options, meta] = value.fields(["group", "name", "options", "_meta"])?;
    let options = lists::skipped(
        Value::required(options, "group.options")?,
        |value| Ok(lists::shape(choice_shape(value.clone()))?.is_some()),
        choice,
    )?;
    Ok(nocterm_ai::model::native::controls::config_group(
        nocterm_ai::model::native::controls::config_group_id(required_text(id)?.text()?)?,
        required_text(name)?.text()?,
        options,
        content::extensions(meta, &value)?,
    )?)
}
