//! Tool fields are constructed directly from the original controlled source.
use super::{content, error::DecodeError, lists, value::Value};
use nocterm_ai::model::{
    extensions::Extensions,
    json::{OwnedJson, deferred::JsonDecodeError},
    native::{NativeList, NativeOwned},
    shared_display::SharedDisplay,
    tools::*,
};
type Error = DecodeError<JsonDecodeError>;

pub(super) fn record(value: Value<'_>) -> Result<NativeOwned<ToolRecord>, Error> {
    let [
        id,
        title,
        name,
        kind,
        status,
        output,
        locations,
        arguments,
        result,
        meta,
    ] = value.fields([
        "toolCallId",
        "title",
        "name",
        "kind",
        "status",
        "content",
        "locations",
        "rawInput",
        "rawOutput",
        "_meta",
    ])?;
    let id = nocterm_ai::model::native::tools::tool_id(required_text(id)?.text()?)?;
    let title = required_text(title)?.text()?;
    let name = content::optional_text(name, &value)?;
    let arguments = optional_json(arguments, &value)?;
    let result = optional_json(result, &value)?;
    let (claims, literal) = evidence(
        &value,
        name.as_optional_str(),
        Some(title.as_str()),
        arguments.as_optional_json(),
    )?;
    let header = nocterm_ai::model::native::tools::header(
        id,
        title,
        name,
        kind_value(kind)?.unwrap_or(ToolKind::Other),
        status_value(status)?.unwrap_or(ToolStatus::Pending),
    )?;
    let output = match output {
        Some(output) => outputs(output)?,
        None => NativeList::new(value.ledger(), 0)?.finish(),
    };
    let locations = match locations {
        Some(locations) => location_list(locations)?,
        None => NativeList::new(value.ledger(), 0)?.finish(),
    };
    Ok(nocterm_ai::model::native::tools::with_literal(
        nocterm_ai::model::native::claims::record_with_claims(
            nocterm_ai::model::native::tools::record(
                header,
                output,
                locations,
                arguments,
                result,
                live_meta(meta, &value)?,
                NativeOwned::<SharedDisplay>::absent(value.ledger())?,
            )?,
            claims,
        )?,
        literal,
    ))
}
pub(super) fn patch(value: Value<'_>) -> Result<NativeOwned<ToolPatch>, Error> {
    if !value.raw().starts_with('{') {
        return Err(DecodeError::Field("tool patch object"));
    }
    let [
        id,
        title,
        name,
        kind,
        status,
        output,
        locations,
        arguments,
        result,
        meta,
    ] = value.fields([
        "toolCallId",
        "title",
        "name",
        "kind",
        "status",
        "content",
        "locations",
        "rawInput",
        "rawOutput",
        "_meta",
    ])?;
    let title = optional_text(title)?;
    let name = optional_text(name)?;
    let arguments = patch_json(arguments)?;
    let result = patch_json(result)?;
    let (claims, literal) = evidence(
        &value,
        name.as_ref().map(NativeOwned::as_str),
        title.as_ref().map(NativeOwned::as_str),
        arguments.as_ref().map(NativeOwned::get),
    )?;
    let mut patch = nocterm_ai::model::native::patch::tool_patch(
        nocterm_ai::model::native::tools::tool_id(required_text(id)?.text()?)?,
        kind_value(kind)?,
        status_value(status)?,
    );
    if let Some(title) = title {
        patch = nocterm_ai::model::native::patch::with_title(patch, title)?;
    }
    if let Some(name) = name {
        patch = nocterm_ai::model::native::patch::with_name(patch, name)?;
    }
    if let Some(output) = output.filter(|value| value.raw().starts_with('[')) {
        patch = nocterm_ai::model::native::patch::with_output(patch, outputs(output)?)?;
    }
    if let Some(locations) = locations.filter(|value| value.raw().starts_with('[')) {
        patch = nocterm_ai::model::native::patch::with_locations(patch, location_list(locations)?)?;
    }
    if let Some(arguments) = arguments {
        patch = nocterm_ai::model::native::patch::with_arguments(patch, arguments)?;
    }
    if let Some(result) = result {
        patch = nocterm_ai::model::native::patch::with_result(patch, result)?;
    }
    Ok(nocterm_ai::model::native::patch::with_literal(
        nocterm_ai::model::native::claims::patch_with_claims(
            nocterm_ai::model::native::claims::with_extensions(patch, live_meta(meta, &value)?)?,
            claims,
        )?,
        literal,
    ))
}
fn optional_text(value: Option<Value<'_>>) -> Result<Option<NativeOwned<String>>, Error> {
    value
        .filter(|value| value.raw().starts_with('"'))
        .map(|value| value.text())
        .transpose()
}
fn patch_json(value: Option<Value<'_>>) -> Result<Option<NativeOwned<OwnedJson>>, Error> {
    value
        .filter(|value| value.raw() != "null")
        .map(|value| {
            Ok(nocterm_ai::model::native::content::json(
                value.literal()?,
                value.ledger(),
            )?)
        })
        .transpose()
}
fn required_text(value: Option<Value<'_>>) -> Result<Value<'_>, Error> {
    let value = Value::required(value, "tool text")?;
    if !value.raw().starts_with('"') {
        return Err(DecodeError::Field("tool text"));
    }
    Ok(value)
}
fn kind_value(value: Option<Value<'_>>) -> Result<Option<ToolKind>, Error> {
    let Some(value) = value.filter(|value| value.raw().starts_with('"')) else {
        return Ok(None);
    };
    for (name, kind) in [
        ("read", ToolKind::Read),
        ("edit", ToolKind::Edit),
        ("delete", ToolKind::Delete),
        ("move", ToolKind::Move),
        ("search", ToolKind::Search),
        ("execute", ToolKind::Execute),
        ("think", ToolKind::Think),
        ("fetch", ToolKind::Fetch),
        ("switch_mode", ToolKind::ChangeMode),
    ] {
        if value.equals(name)? {
            return Ok(Some(kind));
        }
    }
    Ok(Some(ToolKind::Other))
}
fn status_value(value: Option<Value<'_>>) -> Result<Option<ToolStatus>, Error> {
    let Some(value) = value else {
        return Ok(None);
    };
    for (name, status) in [
        ("pending", ToolStatus::Pending),
        ("in_progress", ToolStatus::Running),
        ("completed", ToolStatus::Succeeded),
        ("failed", ToolStatus::Failed),
    ] {
        if value.equals(name)? {
            return Ok(Some(status));
        }
    }
    Ok(None)
}
fn optional_json(
    value: Option<Value<'_>>,
    fallback: &Value<'_>,
) -> Result<NativeOwned<Option<OwnedJson>>, Error> {
    match value.filter(|value| value.raw() != "null") {
        Some(value) => Ok(NativeOwned::supplied(
            nocterm_ai::model::native::content::json(value.literal()?, value.ledger())?,
        )),
        None => Ok(NativeOwned::<OwnedJson>::absent(fallback.ledger())?),
    }
}
fn live_meta(
    value: Option<Value<'_>>,
    fallback: &Value<'_>,
) -> Result<NativeOwned<Option<Extensions>>, Error> {
    match value.filter(|value| value.raw().starts_with('{')) {
        Some(value) => Ok(NativeOwned::supplied(
            nocterm_ai::model::native::content::extensions(
                value.literal_excluding(nocterm_ai::tool_display::META_KEY)?,
                value.ledger(),
            )?,
        )),
        None => Ok(NativeOwned::<Extensions>::absent(fallback.ledger())?),
    }
}
fn evidence(
    value: &Value<'_>,
    name: Option<&str>,
    title: Option<&str>,
    input: Option<&OwnedJson>,
) -> Result<
    (
        NativeOwned<BridgeClaims>,
        nocterm_ai::model::literal::LiteralEvidence,
    ),
    Error,
> {
    use crate::mapping::identity::{self, ClaimProjection};
    let input = input.map(OwnedJson::as_value);
    let literal = identity::literal(name, title, input);
    let [name, title, input] = identity::claims_borrowed(name, title, input);
    let construct = |projection| match projection {
        ClaimProjection::Absent => Ok(nocterm_ai::model::native::claims::absent_claim(
            value.ledger(),
        )?),
        ClaimProjection::Invalid => Ok(nocterm_ai::model::native::claims::invalid_claim(
            value.ledger(),
        )?),
        ClaimProjection::Known { registration, tool } => {
            Ok::<_, Error>(nocterm_ai::model::native::claims::known(
                value.copy_text(registration)?,
                value.copy_text(tool)?,
            )?)
        }
    };
    Ok((
        nocterm_ai::model::native::claims::claims(
            construct(name)?,
            construct(title)?,
            construct(input)?,
            NativeOwned::<String>::absent(value.ledger())?,
        )?,
        literal,
    ))
}
fn location_shape(value: Value<'_>) -> Result<(), Error> {
    let [path, _line, _meta] = value.fields(["path", "line", "_meta"])?;
    required_text(path)?;
    Ok(())
}
fn location(value: Value<'_>) -> Result<NativeOwned<Location>, Error> {
    let [path, line, meta] = value.fields(["path", "line", "_meta"])?;
    let line = line.and_then(|value| value.raw().parse::<u32>().ok());
    Ok(nocterm_ai::model::native::tools::location(
        nocterm_ai::model::native::tools::path(required_text(path)?.text()?)?,
        line,
        content::extensions(meta, &value)?,
    )?)
}
fn location_list(value: Value<'_>) -> Result<NativeOwned<Vec<Location>>, Error> {
    lists::skipped(
        value,
        |value| Ok(lists::shape(location_shape(value.clone()))?.is_some()),
        location,
    )
}
fn outputs(value: Value<'_>) -> Result<NativeOwned<Vec<ToolOutput>>, Error> {
    lists::skipped(
        value,
        |value| Ok(lists::shape(output_shape(value.clone()))?.is_some()),
        output,
    )
}
fn output_shape(value: Value<'_>) -> Result<(), Error> {
    if !value.raw().starts_with('{') {
        return Err(DecodeError::Field("tool output object"));
    }
    let [kind, content, path, replacement, id] =
        value.fields(["type", "content", "path", "newText", "terminalId"])?;
    let kind = required_text(kind)?;
    if kind.equals("content")? {
        content::shape(Value::required(content, "tool.content")?)?;
    } else if kind.equals("diff")? {
        required_text(path)?;
        required_text(replacement)?;
    } else if kind.equals("terminal")? {
        required_text(id)?;
    } else {
        return Err(DecodeError::Field("tool output type"));
    }
    Ok(())
}
fn output(value: Value<'_>) -> Result<NativeOwned<ToolOutput>, Error> {
    let [kind, content, path, previous, replacement, id, meta] = value.fields([
        "type",
        "content",
        "path",
        "oldText",
        "newText",
        "terminalId",
        "_meta",
    ])?;
    let kind = required_text(kind)?;
    let extensions = content::extensions(meta, &value)?;
    if kind.equals("content")? {
        Ok(nocterm_ai::model::native::tools::content_output(
            content::content(Value::required(content, "tool.content")?)?,
            extensions,
        )?)
    } else if kind.equals("diff")? {
        Ok(nocterm_ai::model::native::tools::diff_output(
            nocterm_ai::model::native::tools::diff(
                nocterm_ai::model::native::tools::path(required_text(path)?.text()?)?,
                content::optional_text(previous, &value)?,
                required_text(replacement)?.text()?,
                extensions,
            )?,
        ))
    } else {
        Ok(nocterm_ai::model::native::tools::terminal_output(
            nocterm_ai::model::native::tools::terminal_id(required_text(id)?.text()?)?,
            extensions,
        )?)
    }
}
