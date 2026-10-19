//! Controlled outgoing bytes are prepaid before serde writes their backing.
use super::{budget::SourceBudget, error::DecodeError};
use agent_client_protocol::managed::{
    Buffer, Envelope, ManagedError, MessageId, MessageKind, Payload, Source,
};
use nocterm_ai::model::json::deferred::JsonDecodeError;
use serde::Serialize;
use std::io;
mod content;
mod replies;
mod requests;
type Error = DecodeError<JsonDecodeError>;
struct Sink {
    bytes: Buffer<u8, SourceBudget>,
    refusal: Option<ManagedError<JsonDecodeError>>,
}
impl io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_bytes(bytes).map_err(|error| {
            self.refusal = Some(error);
            io::Error::from(io::ErrorKind::OutOfMemory)
        })?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Sink {
    fn value(&mut self, value: &impl Serialize) -> Result<(), Error> {
        let bytes = nocterm_ai::history::measure_json(value)?.encoded_bytes;
        self.bytes.reserve(bytes)?;
        if serde_json::to_writer(&mut *self, value).is_err() {
            return Err(match self.refusal.take() {
                Some(error) => DecodeError::Allocation(error),
                None => DecodeError::Field("outgoing native value"),
            });
        }
        Ok(())
    }
    fn literal(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.bytes.extend_bytes(bytes)?;
        Ok(())
    }
}
/// Closed field serializers determine the exact source spans while writing.
/// Incoming parser recursion rules do not constrain outgoing neutral JSON.
fn call(
    budget: &SourceBudget,
    id: Option<&MessageId<SourceBudget>>,
    method: &str,
    params: impl Serialize,
) -> Result<Envelope<SourceBudget>, Error> {
    let mut sink = Sink {
        bytes: Source::buffer(budget.clone()),
        refusal: None,
    };
    sink.literal(br#"{"jsonrpc":"2.0""#)?;
    if let Some(id) = id {
        sink.literal(b",\"id\":")?;
        sink.value(&Id(id))?;
    }
    sink.literal(b",\"method\":")?;
    sink.value(&method)?;
    sink.literal(b",\"params\":")?;
    let start = sink.bytes.len();
    sink.value(&params)?;
    let end = sink.bytes.len();
    sink.literal(b"}")?;
    let source = Source::from_buffer(sink.bytes)?;
    let method = Source::copy(method, budget.clone())?;
    Ok(Envelope::validated(
        if id.is_some() {
            MessageKind::Request
        } else {
            MessageKind::Notification
        },
        id.cloned(),
        Some(method.full()),
        Payload::Supplied(source.span(start..end)?),
        source.full(),
    )?)
}
pub(super) use requests::*;

struct Id<'a>(&'a MessageId<SourceBudget>);
impl Serialize for Id<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            MessageId::Null => serializer.serialize_none(),
            MessageId::Number(value) => serializer.serialize_i64(*value),
            MessageId::String(value) => serializer.serialize_str(value.text()),
        }
    }
}
pub(super) use replies::{encode, fault, result};
