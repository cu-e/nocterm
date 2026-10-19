//! Closed permission shape validation preserves the original serde diagnostics.
use super::{
    budget::SourceBudget, diagnostic_text, error::DecodeError, members::Members, value::Value,
};
use agent_client_protocol::managed::{Source, Span};
use nocterm_ai::model::{
    auth::PermissionEffect, json::deferred::JsonDecodeError, native::NativeOwned,
};
mod diagnostics;
type Error = DecodeError<JsonDecodeError>;
enum Fault<'a> {
    Missing(&'static str),
    Type(Option<Value<'a>>, &'static str),
    Length(usize, &'static str, usize),
    Extra(usize),
    SingleKey,
    Variant(NativeOwned<String>),
}
enum Problem<'a> {
    Schema(Fault<'a>),
    Native(Error),
}
impl<'a> From<Error> for Problem<'a> {
    fn from(error: Error) -> Self {
        Self::Native(error)
    }
}
impl<'a> From<Fault<'a>> for Problem<'a> {
    fn from(error: Fault<'a>) -> Self {
        Self::Schema(error)
    }
}
struct Record<'a, const N: usize> {
    fields: [Option<Value<'a>>; N],
    order: [Option<usize>; N],
    sequence: bool,
    len: usize,
    name: &'static str,
}
impl<'a, const N: usize> Record<'a, N> {
    fn new(value: Value<'a>, keys: [&str; N], name: &'static str) -> Result<Self, Problem<'a>> {
        if value.raw().starts_with('{') {
            let fields = value.fields(keys)?;
            let mut order = [None; N];
            let mut used = 0;
            let members =
                Members::new(value.source.text(), value.range.clone(), value.budget, None)?;
            let mut index = 0;
            while let Some(member) = members.get(index) {
                let key = Value::new(value.source, member.key.clone(), value.budget);
                for (ordinal, name) in keys.iter().enumerate() {
                    if key.equals(name)? {
                        order[used] = Some(ordinal);
                        used += 1;
                        break;
                    }
                }
                index += 1;
            }
            return Ok(Self {
                fields,
                order,
                sequence: false,
                len: N,
                name,
            });
        }
        if !value.raw().starts_with('[') {
            return Err(Fault::Type(Some(value), name).into());
        }
        let len = value.array_len()?;
        let mut elements = value.elements()?;
        let mut fields = std::array::from_fn(|_| None);
        for field in &mut fields {
            *field = elements.next()?;
        }
        Ok(Self {
            fields,
            order: std::array::from_fn(|index| (index < len).then_some(index)),
            sequence: true,
            len,
            name,
        })
    }
    fn required(&self, index: usize, name: &'static str) -> Result<Value<'a>, Problem<'a>> {
        self.fields[index].clone().ok_or_else(|| {
            if self.sequence {
                Fault::Length(self.len, self.name, N).into()
            } else {
                Fault::Missing(name).into()
            }
        })
    }
    fn finish(&self) -> Result<(), Problem<'a>> {
        if self.sequence && self.len > N {
            Err(Fault::Extra(self.len).into())
        } else {
            Ok(())
        }
    }
}
fn string<'a>(value: Value<'a>) -> Result<(), Problem<'a>> {
    if value.raw().starts_with('"') {
        Ok(())
    } else {
        Err(Fault::Type(Some(value), "a string").into())
    }
}
fn tool<'a>(value: Value<'a>) -> Result<(), Problem<'a>> {
    if !value.raw().starts_with('{') {
        return Err(Fault::Type(Some(value), "struct ToolCallUpdate").into());
    }
    let [id] = value.fields(["toolCallId"])?;
    string(id.ok_or(Fault::Missing("toolCallId"))?)
}
fn request<'a>(value: Option<Value<'a>>) -> Result<(), Problem<'a>> {
    let value = value.ok_or(Fault::Type(None, "struct RequestPermissionRequest"))?;
    let record = Record::new(
        value,
        ["sessionId", "toolCall", "options", "_meta"],
        "struct RequestPermissionRequest",
    )?;
    for index in record.order.into_iter().flatten() {
        let Some(value) = record.fields[index].clone() else {
            continue;
        };
        match index {
            0 => string(value)?,
            1 => tool(value)?,
            2 => options(value)?,
            _ => {}
        }
    }
    for (index, name) in [(0, "sessionId"), (1, "toolCall"), (2, "options")] {
        record.required(index, name)?;
    }
    record.finish()
}
fn options<'a>(value: Value<'a>) -> Result<(), Problem<'a>> {
    if !value.raw().starts_with('[') {
        return Err(Fault::Type(Some(value), "a sequence").into());
    }
    let mut elements = value.elements()?;
    while let Some(value) = elements.next()? {
        option(value)?;
    }
    Ok(())
}
fn option<'a>(value: Value<'a>) -> Result<(), Problem<'a>> {
    let record = Record::new(
        value,
        ["optionId", "name", "kind", "_meta"],
        "struct PermissionOption",
    )?;
    for index in record.order.into_iter().flatten() {
        let Some(value) = record.fields[index].clone() else {
            continue;
        };
        match index {
            0 | 1 => string(value)?,
            2 => {
                kind(value)?;
            }
            _ => {}
        }
    }
    for (index, name) in [(0, "optionId"), (1, "name"), (2, "kind")] {
        record.required(index, name)?;
    }
    record.finish()
}
fn kind<'a>(value: Value<'a>) -> Result<PermissionEffect, Problem<'a>> {
    let (name, unit) = if value.raw().starts_with('"') {
        (value.text()?, None)
    } else if value.raw().starts_with('{') {
        let members = Members::new(value.source.text(), value.range.clone(), value.budget, None)?;
        let member = members
            .get(0)
            .filter(|_| members.get(1).is_none())
            .ok_or(Fault::SingleKey)?;
        (
            Value::new(value.source, member.key.clone(), value.budget).text()?,
            Some(Value::new(value.source, member.value.clone(), value.budget)),
        )
    } else {
        return Err(Fault::Type(Some(value), "string or map").into());
    };
    let effect = match name.as_str() {
        "allow_once" => PermissionEffect::AllowOnce,
        "allow_always" => PermissionEffect::AllowRemembered,
        "reject_once" => PermissionEffect::DenyOnce,
        "reject_always" => PermissionEffect::DenyRemembered,
        _ => return Err(Fault::Variant(name).into()),
    };
    if let Some(unit) = unit
        && unit.raw() != "null"
    {
        return Err(Fault::Type(Some(unit), "unit").into());
    }
    Ok(effect)
}
pub(super) fn effect(value: Value<'_>) -> Result<PermissionEffect, Error> {
    match kind(value) {
        Ok(effect) => Ok(effect),
        Err(Problem::Native(error)) => Err(error),
        Err(Problem::Schema(_)) => Err(DecodeError::Field("permission kind")),
    }
}
pub(super) fn diagnostic(
    span: Option<&Span<SourceBudget>>,
    budget: &SourceBudget,
) -> Result<Option<Source<SourceBudget>>, Error> {
    let value = span
        .map(|span| Value::from_span(span, budget))
        .transpose()?;
    match request(value) {
        Ok(()) => Ok(None),
        Err(Problem::Native(error)) => Err(error),
        Err(Problem::Schema(fault)) => Ok(Some(diagnostics::render(fault, budget)?)),
    }
}

pub(super) use diagnostics::response;

#[cfg(test)]
mod tests;
