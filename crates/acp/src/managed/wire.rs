//! Controlled JSON-RPC envelopes retain original source spans through SDK routing.
use super::{error::DecodeError, fields::Fields, scan::Scanner};
use agent_client_protocol::managed::{
    Budget, Buffer, Entry, Envelope, Frame, Malformed, MessageId, MessageKind, Payload, Source,
    Span,
};
use std::ops::Range;

pub(super) fn frame<B: Budget>(
    source: &Source<B>,
    budget: B,
) -> Result<Frame<B>, DecodeError<B::Error>> {
    let mut scanner = Scanner::new(source.text(), budget.clone());
    let range = match scanner.complete() {
        Ok(range) => range,
        Err(DecodeError::Syntax(_)) => {
            return Ok(Frame::Malformed(Malformed::validated(
                source.full(),
                false,
                true,
            )));
        }
        Err(error) => return Err(error),
    };
    if source.text().as_bytes().get(range.start) == Some(&b'[') {
        let mut entries = Buffer::new(budget.clone());
        let mut position = scanner.whitespace(range.start + 1);
        while source.text().as_bytes().get(position) != Some(&b']') {
            let end = scanner.value(position)?;
            let entry = entry(source, position..end, budget.clone())?;
            entries
                .push(entry)
                .map_err(|(error, _)| DecodeError::Allocation(error))?;
            position = scanner.whitespace(end);
            if source.text().as_bytes().get(position) == Some(&b',') {
                position = scanner.whitespace(position + 1);
            }
        }
        if entries.is_empty() {
            Ok(Frame::Malformed(Malformed::validated(
                source.full(),
                false,
                false,
            )))
        } else {
            Ok(Frame::batch(entries)?)
        }
    } else {
        match entry(source, range, budget)? {
            Entry::Message(envelope) => Ok(Frame::Single(envelope)),
            Entry::Malformed(malformed) => Ok(Frame::Malformed(malformed)),
        }
    }
}
fn entry<B: Budget>(
    source: &Source<B>,
    range: Range<usize>,
    budget: B,
) -> Result<Entry<B>, DecodeError<B::Error>> {
    if source.text().as_bytes().get(range.start) != Some(&b'{') {
        return Ok(Entry::Malformed(Malformed::validated(
            source.span(range)?,
            false,
            false,
        )));
    }
    let fields = Fields::new(source.text(), range.clone(), budget.clone())?
        .select(["jsonrpc", "method", "id", "params", "result", "error"])?;
    let response_only = fields[1].is_none() && (fields[4].is_some() || fields[5].is_some());
    match envelope(source, range.clone(), fields, budget) {
        Ok(envelope) => Ok(Entry::Message(envelope)),
        Err(DecodeError::Field(_)) => Ok(Entry::Malformed(Malformed::validated(
            source.span(range)?,
            response_only,
            false,
        ))),
        Err(error) => Err(error),
    }
}
fn envelope<B: Budget>(
    source: &Source<B>,
    range: Range<usize>,
    fields: [Option<Range<usize>>; 6],
    budget: B,
) -> Result<Envelope<B>, DecodeError<B::Error>> {
    let [version, method, id, params, result, error] = fields;
    let scanner = Scanner::new(source.text(), budget.clone());
    if !version.is_some_and(|range| scanner.key_equals(range, "2.0").unwrap_or(false)) {
        return Err(DecodeError::Field("jsonrpc"));
    }
    if let Some(method) = method {
        if result.is_some() || error.is_some() {
            return Err(DecodeError::Field("method/result/error"));
        }
        let method = text(source, method, budget.clone())?;
        let id = id
            .map(|range| identifier(source, range, budget.clone()))
            .transpose()?;
        let payload = match params {
            None => Payload::Absent,
            Some(range) => {
                if !matches!(
                    source.text().as_bytes().get(range.start),
                    Some(b'{' | b'[' | b'n')
                ) {
                    return Err(DecodeError::Field("params"));
                }
                Payload::Supplied(source.span(range)?)
            }
        };
        let kind = if id.is_some() {
            MessageKind::Request
        } else {
            MessageKind::Notification
        };
        Ok(Envelope::validated(
            kind,
            id,
            Some(method),
            payload,
            source.span(range)?,
        )?)
    } else {
        let id = identifier(source, id.ok_or(DecodeError::Field("id"))?, budget.clone())?;
        let (kind, payload) = match (result, error) {
            (Some(result), None) => (MessageKind::Result, result),
            (None, Some(error)) => {
                error_body(source, error.clone(), budget)?;
                (MessageKind::Error, error)
            }
            _ => return Err(DecodeError::Field("result/error")),
        };
        Ok(Envelope::validated(
            kind,
            Some(id),
            None,
            Payload::Supplied(source.span(payload)?),
            source.span(range)?,
        )?)
    }
}
fn identifier<B: Budget>(
    source: &Source<B>,
    range: Range<usize>,
    budget: B,
) -> Result<MessageId<B>, DecodeError<B::Error>> {
    let raw = &source.text()[range.clone()];
    if raw == "null" {
        return Ok(MessageId::Null);
    }
    if raw.starts_with('"') {
        return Ok(MessageId::String(text(source, range, budget)?));
    }
    // serde_json represents negative integer zero as f64; the original typed
    // RequestId then rejects it, even though ordinary i64 parsing accepts it.
    if raw == "-0" {
        return Err(DecodeError::Field("integer id"));
    }
    raw.parse::<i64>()
        .map(MessageId::Number)
        .map_err(|_| DecodeError::Field("id"))
}
fn error_body<B: Budget>(
    source: &Source<B>,
    range: Range<usize>,
    budget: B,
) -> Result<(), DecodeError<B::Error>> {
    if source.text().as_bytes().get(range.start) != Some(&b'{') {
        return Err(DecodeError::Field("error"));
    }
    let [code, message] =
        Fields::new(source.text(), range, budget.clone())?.select(["code", "message"])?;
    let code = code.ok_or(DecodeError::Field("error.code"))?;
    let raw = &source.text()[code];
    if raw == "-0" || raw.parse::<i32>().is_err() {
        return Err(DecodeError::Field("error.code"));
    }
    let message = message.ok_or(DecodeError::Field("error.message"))?;
    if source.text().as_bytes().get(message.start) != Some(&b'"') {
        return Err(DecodeError::Field("error.message"));
    }
    let scanner = Scanner::new(source.text(), budget);
    scanner.string(message.start)?;
    Ok(())
}
pub(super) fn text<B: Budget>(
    source: &Source<B>,
    range: Range<usize>,
    budget: B,
) -> Result<Span<B>, DecodeError<B::Error>> {
    if source.text().as_bytes().get(range.start) != Some(&b'"') {
        return Err(DecodeError::Field("string"));
    }
    let raw = &source.text()[range.clone()];
    if !raw.as_bytes().contains(&b'\\') {
        return Ok(source.span(range.start + 1..range.end - 1)?);
    }
    let scanner = Scanner::new(source.text(), budget.clone());
    let mut bytes = Source::buffer(budget);
    scanner.decoded_string(range, |fragment| {
        bytes.extend_bytes(fragment.as_bytes())?;
        Ok(())
    })?;
    Ok(Source::from_buffer(bytes)?.full())
}
