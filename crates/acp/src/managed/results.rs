//! Used RPC results preserve the pinned schema's null/default/positional rules.
use super::{content, controls, copy, error::DecodeError, lists, value::Value};
use nocterm_ai::model::{
    controls::{ConfigOption, ModeChoice, Modes},
    json::deferred::JsonDecodeError,
    native::NativeOwned,
    session::{PromptOutcome, SessionOpened},
    usage::TokenTotals,
};
type Error = DecodeError<JsonDecodeError>;
fn required_text(value: Option<Value<'_>>) -> Result<Value<'_>, Error> {
    let value = Value::required(value, "session text")?;
    if !value.raw().starts_with('"') {
        return Err(DecodeError::Field("session text"));
    }
    Ok(value)
}
fn mode_shape(value: Value<'_>) -> Result<(), Error> {
    let [id, name, _description, _meta] = value.fields(["id", "name", "description", "_meta"])?;
    required_text(id)?;
    required_text(name)?;
    Ok(())
}
fn mode(value: Value<'_>) -> Result<NativeOwned<ModeChoice>, Error> {
    let [id, name, description, meta] = value.fields(["id", "name", "description", "_meta"])?;
    Ok(nocterm_ai::model::native::controls::mode_choice(
        nocterm_ai::model::native::controls::mode_id(required_text(id)?.text()?)?,
        required_text(name)?.text()?,
        content::optional_text(description, &value)?,
        content::extensions(meta, &value)?,
    )?)
}
fn modes_shape(value: Value<'_>) -> Result<(), Error> {
    let [current, choices, _meta] = value.fields(["currentModeId", "availableModes", "_meta"])?;
    required_text(current)?;
    Value::required(choices, "availableModes")?;
    Ok(())
}
fn modes(value: Value<'_>) -> Result<NativeOwned<Modes>, Error> {
    let [current, choices, meta] = value.fields(["currentModeId", "availableModes", "_meta"])?;
    let choices = lists::skipped(
        Value::required(choices, "availableModes")?,
        |value| Ok(lists::shape(mode_shape(value.clone()))?.is_some()),
        mode,
    )?;
    Ok(nocterm_ai::model::native::controls::modes(
        nocterm_ai::model::native::controls::mode_id(required_text(current)?.text()?)?,
        choices,
        content::extensions(meta, &value)?,
    )?)
}
#[cfg(test)]
pub(super) fn opened(
    value: Value<'_>,
    restored_id: Option<&str>,
) -> Result<NativeOwned<SessionOpened>, Error> {
    opened_with_models(value, restored_id, None)
}
#[cfg(test)]
pub(super) fn opened_with_models(
    value: Value<'_>,
    restored_id: Option<&str>,
    models: Option<&mut super::models::Models>,
) -> Result<NativeOwned<SessionOpened>, Error> {
    let prepared = prepared_opened(value, restored_id)?;
    let (opened, selector) = prepared.into_parts();
    if let Some(models) = models {
        install_opened(models, opened.get(), selector.as_ref())?;
    }
    Ok(opened)
}
pub(super) struct PreparedOpened {
    opened: NativeOwned<SessionOpened>,
    selector: Option<NativeOwned<String>>,
}
impl PreparedOpened {
    pub(super) fn into_parts(self) -> (NativeOwned<SessionOpened>, Option<NativeOwned<String>>) {
        (self.opened, self.selector)
    }
}
pub(super) fn install_opened(
    models: &mut super::models::Models,
    opened: &SessionOpened,
    selector: Option<&NativeOwned<String>>,
) -> Result<(), Error> {
    if let Some(selector) = selector {
        models.install(
            opened.session.as_str(),
            selector.as_str(),
            opened.configuration.as_deref().unwrap_or(&[]),
        )?;
    } else {
        models.remove(opened.session.as_str());
    }
    Ok(())
}
pub(super) fn prepared_opened(
    value: Value<'_>,
    restored_id: Option<&str>,
) -> Result<PreparedOpened, Error> {
    let [session, state, options, _meta] =
        value.fields(["sessionId", "modes", "configOptions", "_meta"])?;
    // The actual old Models.session path injects the requested ID only into
    // object results; null/positional responses are not rewritten.
    let session = match restored_id.filter(|_| value.raw().starts_with('{')) {
        Some(id) => copy::text(value.budget, id)?,
        None => required_text(session)?.text()?,
    };
    if session.as_str().is_empty() {
        return Err(DecodeError::Field("nonempty session ID"));
    }
    let modes = match state {
        Some(state) if lists::shape(modes_shape(state.clone()))?.is_some() => {
            NativeOwned::supplied(modes(state)?)
        }
        _ => NativeOwned::absent(value.ledger())?,
    };
    let options = match options.filter(|value| value.raw().starts_with('[')) {
        Some(options) => Some(controls::options(options)?),
        None => None,
    };
    let normalized = super::legacy_models::normalize(
        &value,
        options
            .as_ref()
            .map(|options| options.get().as_slice())
            .unwrap_or(&[]),
    )?;
    let (selector, options) = match normalized {
        Some((selector, options)) => (Some(selector), Some(options)),
        None => (None, options),
    };
    let options = match options {
        Some(options) => NativeOwned::supplied(options),
        None => NativeOwned::absent(value.ledger())?,
    };
    let opened = nocterm_ai::model::native::session::opened(
        nocterm_ai::model::native::controls::session_id(session)?,
        modes,
        options,
    )?;
    Ok(PreparedOpened { opened, selector })
}

fn integer(value: Option<Value<'_>>, label: &'static str) -> Result<u64, Error> {
    Value::required(value, label)?
        .raw()
        .parse()
        .map_err(|_| DecodeError::Field(label))
}
fn totals(value: Value<'_>) -> Result<NativeOwned<TokenTotals>, Error> {
    let [total, input, output, thought, read, write, _meta] = value.fields([
        "totalTokens",
        "inputTokens",
        "outputTokens",
        "thoughtTokens",
        "cachedReadTokens",
        "cachedWriteTokens",
        "_meta",
    ])?;
    Ok(nocterm_ai::model::native::session::totals(
        integer(total, "totalTokens")?,
        integer(input, "inputTokens")?,
        integer(output, "outputTokens")?,
        thought.and_then(|value| value.raw().parse().ok()),
        read.and_then(|value| value.raw().parse().ok()),
        write.and_then(|value| value.raw().parse().ok()),
        value.ledger(),
    )?)
}
fn totals_shape(value: Value<'_>) -> Result<(), Error> {
    let [total, input, output, _thought, _read, _write, _meta] = value.fields([
        "totalTokens",
        "inputTokens",
        "outputTokens",
        "thoughtTokens",
        "cachedReadTokens",
        "cachedWriteTokens",
        "_meta",
    ])?;
    integer(total, "totalTokens")?;
    integer(input, "inputTokens")?;
    integer(output, "outputTokens")?;
    Ok(())
}
pub(super) fn outcome(value: Value<'_>) -> Result<NativeOwned<PromptOutcome>, Error> {
    let [ending, usage, _meta] = value.fields(["stopReason", "usage", "_meta"])?;
    let ending = required_text(ending)?;
    let ending = if ending.equals("end_turn")? {
        nocterm_ai::model::native::session::finished(value.ledger())?
    } else if ending.equals("max_tokens")? {
        nocterm_ai::model::native::session::token_limit(value.ledger())?
    } else if ending.equals("max_turn_requests")? {
        nocterm_ai::model::native::session::request_limit(value.ledger())?
    } else if ending.equals("refusal")? {
        nocterm_ai::model::native::session::refused(value.ledger())?
    } else if ending.equals("cancelled")? {
        nocterm_ai::model::native::session::cancelled(value.ledger())?
    } else {
        return Err(DecodeError::Field("stopReason"));
    };
    let usage = match usage {
        Some(usage) if lists::shape(totals_shape(usage.clone()))?.is_some() => {
            NativeOwned::supplied(totals(usage)?)
        }
        _ => NativeOwned::absent(value.ledger())?,
    };
    Ok(nocterm_ai::model::native::session::outcome(ending, usage)?)
}
pub(super) fn configuration(value: Value<'_>) -> Result<NativeOwned<Vec<ConfigOption>>, Error> {
    let [options, _meta] = value.fields(["configOptions", "_meta"])?;
    controls::options(Value::required(options, "configOptions")?)
}
pub(super) fn empty(value: Value<'_>) -> Result<(), Error> {
    if value.raw() != "null" {
        value.fields(["_meta"])?;
    }
    Ok(())
}
