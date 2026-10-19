//! Original serde messages and invalid_params data write into paid wire owners.
use super::{Error, Fault, Source, SourceBudget, Value, diagnostic_text};
use agent_client_protocol::managed::{Envelope, MessageId, Span};
use nocterm_ai::model::json::OwnedJson;
use serde::de::Unexpected;
pub(super) fn render(
    fault: Fault<'_>,
    budget: &SourceBudget,
) -> Result<Source<SourceBudget>, Error> {
    Ok(match fault {
        Fault::Missing(field) => {
            diagnostic_text::format(format_args!("missing field `{field}`"), budget)?
        }
        Fault::Length(len, name, count) => diagnostic_text::format(
            format_args!("invalid length {len}, expected {name} with {count} elements"),
            budget,
        )?,
        Fault::Extra(len) => diagnostic_text::format(
            format_args!("invalid length {len}, expected fewer elements in array"),
            budget,
        )?,
        Fault::SingleKey => Source::copy(
            "invalid value: map, expected map with a single key",
            budget.clone(),
        )?,
        Fault::Variant(name) => diagnostic_text::format(
            format_args!(
                "unknown variant `{}`, expected one of `allow_once`, `allow_always`, `reject_once`, `reject_always`",
                name.as_str()
            ),
            budget,
        )?,
        Fault::Type(value, expected) => unexpected(value, expected, budget)?,
    })
}
fn unexpected(
    value: Option<Value<'_>>,
    expected: &str,
    budget: &SourceBudget,
) -> Result<Source<SourceBudget>, Error> {
    let text = match &value {
        Some(value) if value.raw().starts_with('"') => Some(value.text()?),
        _ => None,
    };
    let raw = value.as_ref().map_or("null", Value::raw);
    let actual = if let Some(text) = &text {
        Unexpected::Str(text.as_str())
    } else {
        match raw.as_bytes().first() {
            Some(b'[') => Unexpected::Seq,
            Some(b'{') => Unexpected::Map,
            Some(b't') => Unexpected::Bool(true),
            Some(b'f') => Unexpected::Bool(false),
            Some(b'n') => Unexpected::Other("null"),
            _ => {
                if let Ok(number) = raw.parse::<i64>() {
                    Unexpected::Signed(number)
                } else if let Ok(number) = raw.parse::<u64>() {
                    Unexpected::Unsigned(number)
                } else {
                    Unexpected::Float(
                        raw.parse()
                            .map_err(|_| super::DecodeError::Field("number"))?,
                    )
                }
            }
        }
    };
    Ok(diagnostic_text::format(
        format_args!("invalid type: {actual}, expected {expected}"),
        budget,
    )?)
}
#[derive(serde::Serialize)]
struct Data<'a> {
    error: &'a str,
    json: &'a OwnedJson,
    phase: &'static str,
}
pub(in crate::managed) fn response(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    span: Option<&Span<SourceBudget>>,
    error: &Source<SourceBudget>,
) -> Result<Envelope<SourceBudget>, Error> {
    let json = match span {
        Some(span) => nocterm_ai::model::native::content::json(
            Value::from_span(span, budget)?.literal()?,
            budget.ledger(),
        )?,
        None => nocterm_ai::model::native::content::null_json(budget.ledger())?,
    };
    super::super::outgoing::fault(
        budget,
        id,
        -32602,
        "Invalid params",
        Some(&Data {
            error: error.text(),
            json: json.get(),
            phase: "deserialization",
        }),
    )
}
