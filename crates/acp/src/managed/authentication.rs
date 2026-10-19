//! Authentication and permission decode directly into original paid neutral values.
use super::{
    budget::SourceBudget, content, error::DecodeError, lists, members::Members, tools, value::Value,
};
use agent_client_protocol::managed::Span;
use nocterm_ai::model::{
    auth::*,
    json::deferred::JsonDecodeError,
    native::{NativeList, NativeOwned},
};

type Error = DecodeError<JsonDecodeError>;
fn required_text(value: Option<Value<'_>>) -> Result<Value<'_>, Error> {
    let value = Value::required(value, "authentication text")?;
    if !value.raw().starts_with('"') {
        return Err(DecodeError::Field("authentication text"));
    }
    Ok(value)
}
fn method_shape(value: Value<'_>) -> Result<(), Error> {
    let [id, name, _description, _meta] = value.fields(["id", "name", "description", "_meta"])?;
    required_text(id)?;
    required_text(name)?;
    Ok(())
}
pub(super) fn methods(value: Value<'_>) -> Result<NativeOwned<Vec<AuthMethod>>, Error> {
    lists::skipped(
        value,
        |value| Ok(lists::shape(method_shape(value.clone()))?.is_some()),
        method,
    )
}
pub(super) fn method(value: Value<'_>) -> Result<NativeOwned<AuthMethod>, Error> {
    let terminal = if value.raw().starts_with('{') {
        let [kind] = value.fields(["type"])?;
        kind.map(|kind| kind.equals("terminal"))
            .transpose()?
            .unwrap_or(false)
    } else {
        false
    };
    let (id, name, description, args, env) = if terminal {
        let [id, name, description, args, env, _meta] =
            value.fields(["id", "name", "description", "args", "env", "_meta"])?;
        (id, name, description, args, env)
    } else {
        let [id, name, description, _meta] =
            value.fields(["id", "name", "description", "_meta"])?;
        (id, name, description, None, None)
    };
    let flow = if terminal {
        let args = match args {
            Some(args) => lists::skipped(
                args,
                |value| Ok(value.raw().starts_with('"')),
                |value| value.text(),
            )?,
            None => NativeList::new(value.ledger(), 0)?.finish(),
        };
        nocterm_ai::model::native::auth::terminal_setup_flow(
            nocterm_ai::model::native::auth::terminal_setup(args, environment(env, &value)?)?,
        )
    } else {
        nocterm_ai::model::native::auth::agent_managed(value.ledger())?
    };
    Ok(nocterm_ai::model::native::auth::auth_method(
        nocterm_ai::model::native::auth::auth_method_id(required_text(id)?.text()?)?,
        required_text(name)?.text()?,
        content::optional_text(description, &value)?,
        flow,
    )?)
}
fn environment(
    value: Option<Value<'_>>,
    fallback: &Value<'_>,
) -> Result<NativeOwned<TerminalEnvironment>, Error> {
    let Some(value) = value.filter(|value| value.raw().starts_with('{')) else {
        return Ok(nocterm_ai::model::native::auth::environment(
            NativeList::new(fallback.ledger(), 0)?.finish(),
        )?);
    };
    let members = Members::new(value.source.text(), value.range.clone(), value.budget, None)?;
    let mut count = 0;
    while let Some(member) = members.get(count) {
        if value.source.text().as_bytes().get(member.value.start) != Some(&b'"') {
            // DefaultOnError applies to the WHOLE map after duplicate winners.
            return Ok(nocterm_ai::model::native::auth::environment(
                NativeList::new(value.ledger(), 0)?.finish(),
            )?);
        }
        count += 1;
    }
    let mut result = NativeList::new(value.ledger(), count)?;
    for index in 0..count {
        let member = members
            .get(index)
            .ok_or(DecodeError::Field("environment index"))?;
        let key = Value::new(value.source, member.key.clone(), value.budget).text()?;
        let text = Value::new(value.source, member.value.clone(), value.budget).text()?;
        result.push(nocterm_ai::model::native::auth::environment_pair(
            key, text,
        )?)?;
    }
    Ok(nocterm_ai::model::native::auth::environment(
        result.finish(),
    )?)
}
pub(super) fn permission(
    span: &Span<SourceBudget>,
    budget: &SourceBudget,
) -> Result<NativeOwned<ProviderPermission>, Error> {
    let value = Value::from_span(span, budget)?;
    let [session, tool, options, meta] =
        value.fields(["sessionId", "toolCall", "options", "_meta"])?;
    let options = Value::required(options, "permission.options")?;
    let mut choices = NativeList::new(value.ledger(), options.array_len()?)?;
    let mut elements = options.elements()?;
    while let Some(option) = elements.next()? {
        let [id, name, kind, meta] = option.fields(["optionId", "name", "kind", "_meta"])?;
        let effect = super::permission_schema::effect(Value::required(kind, "kind")?)?;
        choices.push(nocterm_ai::model::native::auth::permission_choice(
            nocterm_ai::model::native::auth::permission_option_id(required_text(id)?.text()?)?,
            required_text(name)?.text()?,
            effect,
            content::extensions(meta, &option)?,
        )?)?;
    }
    Ok(nocterm_ai::model::native::auth::permission(
        nocterm_ai::model::native::controls::session_id(required_text(session)?.text()?)?,
        tools::patch(Value::required(tool, "permission.toolCall")?)?,
        choices.finish(),
        content::extensions(meta, &value)?,
    )?)
}
