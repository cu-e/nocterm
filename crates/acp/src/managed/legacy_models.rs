//! Original bounded legacy selector normalization over authenticated source spans.
use super::{
    configuration_copy, copy, error::DecodeError, members::Key, scan::Scanner, value::Value,
};
use agent_client_protocol::managed::{Buffer, Index};
use nocterm_ai::model::{
    controls::{ConfigCategory, ConfigOption},
    json::deferred::{JsonDecodeError, literal::LiteralBuilder},
    native::{NativeList, NativeOwned},
};
type Error = DecodeError<JsonDecodeError>;
struct Choice<'a> {
    id: Value<'a>,
    name: Value<'a>,
}
fn bounded_text(value: &Value<'_>, name: bool) -> Result<bool, Error> {
    if !value.raw().starts_with('"') {
        return Ok(false);
    }
    let mut bytes = 0usize;
    let mut nonblank = false;
    Scanner::new(value.source.text(), value.budget.clone()).decoded_string(
        value.range.clone(),
        |part| {
            bytes += part.len();
            nonblank |= part.chars().any(|character| !character.is_whitespace());
            Ok(())
        },
    )?;
    Ok(bytes > 0 && bytes <= 1024 && (!name || nonblank))
}
fn selector(value: &Value<'_>, options: &[ConfigOption]) -> Result<NativeOwned<String>, Error> {
    let base = "__nocterm_legacy_model";
    let mut suffix = 0;
    while options.iter().any(|option| {
        let id = option.id.as_str();
        id.len() == base.len() + suffix
            && id.starts_with(base)
            && id[base.len()..].bytes().all(|byte| byte == b'_')
    }) {
        suffix += 1;
    }
    let mut builder = LiteralBuilder::new(value.ledger())?;
    let mut text = builder.exact_text(base.len() + suffix)?;
    text.append(base)?;
    for _ in 0..suffix {
        text.append("_")?;
    }
    builder.string(text.finish()?)?;
    Ok(NativeOwned::text(builder.finish_native()?.into_text()?))
}
pub(super) fn normalize(
    value: &Value<'_>,
    options: &[ConfigOption],
) -> Result<Option<(NativeOwned<String>, NativeOwned<Vec<ConfigOption>>)>, Error> {
    if !value.raw().starts_with('{')
        || options
            .iter()
            .any(|option| option.category == Some(ConfigCategory::Model))
    {
        return Ok(None);
    }
    let [models] = value.fields(["models"])?;
    let Some(models) = models.filter(|item| item.raw().starts_with('{')) else {
        return Ok(None);
    };
    let [current, choices] = models.fields(["currentModelId", "availableModels"])?;
    let Some(current) = current.filter(|item| item.raw().starts_with('"')) else {
        return Ok(None);
    };
    let Some(choices) = choices.filter(|item| item.raw().starts_with('[')) else {
        return Ok(None);
    };
    let count = choices.array_len()?;
    if count > 512 {
        return Ok(None);
    }
    let mut seen = Index::new(value.budget.clone());
    let mut valid = Buffer::new(value.budget.clone());
    let mut elements = choices.elements()?;
    let mut includes_current = false;
    while let Some(choice) = elements.next()? {
        if !choice.raw().starts_with('{') {
            continue;
        }
        let [id, name] = choice.fields(["modelId", "name"])?;
        let (Some(id), Some(name)) = (id, name) else {
            continue;
        };
        if !bounded_text(&id, false)? || !bounded_text(&name, true)? {
            continue;
        }
        let key = Key::new(id.source.text(), id.range.clone(), id.budget)?;
        if seen.get(&key).is_some() {
            continue;
        }
        // Source-key Eq follows decoded Unicode; it does not copy discarded IDs.
        let current_key = Key::new(current.source.text(), current.range.clone(), current.budget)?;
        includes_current |= key == current_key;
        seen.insert(key, ())
            .map_err(|(error, _, _)| DecodeError::Allocation(error))?;
        valid
            .push(Choice { id, name })
            .map_err(|(error, _)| DecodeError::Allocation(error))?;
    }
    if !includes_current {
        return Ok(None);
    }
    let selector = selector(value, options)?;
    let selector_copy = copy::text(value.budget, selector.as_str())?;
    let mut entries = NativeList::new(value.ledger(), valid.len())?;
    for choice in valid.into_values() {
        entries.push(nocterm_ai::model::native::controls::config_choice(
            nocterm_ai::model::native::controls::config_choice_id(choice.id.text()?)?,
            choice.name.text()?,
            NativeOwned::absent(value.ledger())?,
            NativeOwned::absent(value.ledger())?,
        )?)?;
    }
    let result = with_model_option(value, options, selector_copy, &current, entries.finish())?;
    Ok(Some((selector, result)))
}
/// Appends the synthesized model selector after copies of the original options.
fn with_model_option(
    value: &Value<'_>,
    options: &[ConfigOption],
    selector: NativeOwned<String>,
    current: &Value<'_>,
    entries: NativeOwned<Vec<nocterm_ai::model::controls::ConfigChoice>>,
) -> Result<NativeOwned<Vec<ConfigOption>>, Error> {
    let mut result = NativeList::new(value.ledger(), options.len() + 1)?;
    for option in options {
        result.push(configuration_copy::option(value.budget, option, None)?)?;
    }
    result.push(nocterm_ai::model::native::controls::config_option(
        nocterm_ai::model::native::controls::config_id(selector)?,
        copy::text(value.budget, "Model")?,
        NativeOwned::absent(value.ledger())?,
        NativeOwned::supplied(
            nocterm_ai::model::native::controls::known("model", value.ledger())?
                .ok_or(DecodeError::Field("model category"))?,
        ),
        nocterm_ai::model::native::controls::select(
            nocterm_ai::model::native::controls::config_choice_id(current.text()?)?,
            nocterm_ai::model::native::controls::flat(entries),
        )?,
        NativeOwned::absent(value.ledger())?,
    )?)?;
    Ok(result.finish())
}
