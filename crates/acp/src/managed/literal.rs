//! Prepaid iterative construction visits only surviving object values.
use super::{
    budget::SourceBudget, error::DecodeError, members::Members, scan::Scanner, value::Value,
};
use agent_client_protocol::managed::Buffer;
use nocterm_ai::model::json::deferred::{
    JsonDecodeError, literal::LiteralBuilder, native::NativeNode,
};
use std::ops::Range;

type Error = DecodeError<JsonDecodeError>;
#[expect(
    clippy::large_enum_variant,
    reason = "the frame stack is charged per inline frame; boxing would add unpaid allocations"
)]
enum Frame<'a> {
    Object { members: Members<'a>, cursor: usize },
    Array { position: usize, end: usize },
}
pub(super) fn build(value: &Value<'_>, excluded: Option<&str>) -> Result<NativeNode, Error> {
    let mut builder = LiteralBuilder::new(value.ledger())?;
    let mut frames = Buffer::new(value.budget.clone());
    emit(
        value,
        value.range.clone(),
        excluded,
        &mut builder,
        &mut frames,
    )?;
    let mut scanner = Scanner::new(value.source.text(), value.budget.clone());
    while let Some(frame) = frames.as_mut_slice().last_mut() {
        let next = match frame {
            Frame::Object { members, cursor } => match members.get(*cursor) {
                Some(member) => {
                    let key = Value::new(value.source, member.key.clone(), value.budget);
                    builder.key(key.literal_text(&builder)?)?;
                    *cursor += 1;
                    Some(member.value.clone())
                }
                None => None,
            },
            Frame::Array { position, end } => {
                *position = scanner.whitespace(*position);
                if *position >= *end {
                    None
                } else {
                    let start = *position;
                    let finish = scanner.value(start)?;
                    *position = scanner.whitespace(finish);
                    if value.source.text().as_bytes().get(*position) == Some(&b',') {
                        *position += 1;
                    }
                    Some(start..finish)
                }
            }
        };
        if let Some(range) = next {
            emit(value, range, None, &mut builder, &mut frames)?;
        } else {
            builder.end_container()?;
            frames.pop();
        }
    }
    Ok(builder.finish_native()?)
}
fn emit<'a>(
    value: &Value<'a>,
    range: Range<usize>,
    excluded: Option<&str>,
    builder: &mut LiteralBuilder,
    frames: &mut Buffer<Frame<'a>, SourceBudget>,
) -> Result<(), Error> {
    let input = value.source.text();
    let frame = match input.as_bytes().get(range.start) {
        Some(b'{') => {
            let members = Members::new(input, range, value.budget, excluded)?;
            builder.start_object()?;
            Some(Frame::Object { members, cursor: 0 })
        }
        Some(b'[') => {
            builder.start_array()?;
            Some(Frame::Array {
                position: range.start + 1,
                end: range.end - 1,
            })
        }
        Some(b'"') => {
            let text = Value::new(value.source, range, value.budget).literal_text(builder)?;
            builder.string(text)?;
            None
        }
        Some(b'n') => {
            builder.null()?;
            None
        }
        Some(b't') => {
            builder.boolean(true)?;
            None
        }
        Some(b'f') => {
            builder.boolean(false)?;
            None
        }
        Some(b'-' | b'0'..=b'9') => {
            let number = input[range]
                .parse::<serde_json::Number>()
                .map_err(|_| DecodeError::Field("number"))?;
            builder.number(number)?;
            None
        }
        _ => return Err(DecodeError::Syntax(range.start)),
    };
    if let Some(frame) = frame {
        frames
            .push(frame)
            .map_err(|(error, _)| DecodeError::Allocation(error))?;
    }
    Ok(())
}
