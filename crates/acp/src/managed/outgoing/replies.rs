//! Encode the actual SDK output directives without an intermediate JSON graph.
use super::{Error, Id, Sink, SourceBudget};
use agent_client_protocol::managed::{
    Envelope, Fault, MessageId, MessageKind, Outgoing, Payload, Reply, Source,
};
use serde::Serialize;

fn sink(budget: &SourceBudget) -> Sink {
    Sink {
        bytes: Source::buffer(budget.clone()),
        refusal: None,
    }
}
fn response_start(sink: &mut Sink, id: &MessageId<SourceBudget>) -> Result<(), Error> {
    sink.literal(br#"{"jsonrpc":"2.0","id":"#)?;
    sink.value(&Id(id))
}
pub(in crate::managed) fn result(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    value: &impl Serialize,
) -> Result<Envelope<SourceBudget>, Error> {
    let mut sink = sink(budget);
    response_start(&mut sink, id)?;
    sink.literal(b",\"result\":")?;
    let start = sink.bytes.len();
    sink.value(value)?;
    let end = sink.bytes.len();
    sink.literal(b"}")?;
    let source = Source::from_buffer(sink.bytes)?;
    Ok(Envelope::validated(
        MessageKind::Result,
        Some(id.clone()),
        None,
        Payload::Supplied(source.span(start..end)?),
        source.full(),
    )?)
}
struct Abandoned<'a>(&'a str);
impl Serialize for Abandoned<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!(
            "request handler dropped its responder for `{}`",
            self.0
        ))
    }
}
fn reply(sink: &mut Sink, reply: &Reply<SourceBudget>) -> Result<(), Error> {
    let Reply::Fault {
        id,
        fault,
        method,
        original,
    } = reply
    else {
        let Reply::Message(message) = reply else {
            unreachable!()
        };
        return sink.literal(message.original().text().as_bytes());
    };
    response_start(sink, id)?;
    let (code, message) = match fault {
        Fault::Parse => (-32700, "Parse error"),
        Fault::InvalidRequest => (-32600, "Invalid request"),
        Fault::MethodNotFound => (-32601, "Method not found"),
        Fault::Abandoned => (-32603, "Internal error"),
    };
    sink.literal(b",\"error\":{\"code\":")?;
    sink.value(&code)?;
    sink.literal(b",\"message\":")?;
    sink.value(&message)?;
    match fault {
        Fault::Parse => {
            sink.literal(b",\"data\":{\"line\":")?;
            sink.value(&original.as_ref().map_or("", |span| span.text()))?;
            sink.literal(b"}")?;
        }
        Fault::InvalidRequest => {}
        Fault::MethodNotFound | Fault::Abandoned => {
            let method = method.as_ref().map_or("", |span| span.text());
            sink.literal(b",\"data\":")?;
            if *fault == Fault::Abandoned {
                sink.value(&Abandoned(method))?;
            } else {
                sink.value(&method)?;
            }
        }
    }
    sink.literal(b"}}")
}
pub(in crate::managed) fn fault<T: Serialize>(
    budget: &SourceBudget,
    id: &MessageId<SourceBudget>,
    code: i32,
    message: &str,
    data: Option<&T>,
) -> Result<Envelope<SourceBudget>, Error> {
    let mut sink = sink(budget);
    response_start(&mut sink, id)?;
    sink.literal(b",\"error\":")?;
    let start = sink.bytes.len();
    sink.literal(b"{\"code\":")?;
    sink.value(&code)?;
    sink.literal(b",\"message\":")?;
    sink.value(&message)?;
    if let Some(data) = data {
        sink.literal(b",\"data\":")?;
        sink.value(data)?;
    }
    sink.literal(b"}")?;
    let end = sink.bytes.len();
    sink.literal(b"}")?;
    let source = Source::from_buffer(sink.bytes)?;
    Ok(Envelope::validated(
        MessageKind::Error,
        Some(id.clone()),
        None,
        Payload::Supplied(source.span(start..end)?),
        source.full(),
    )?)
}
pub(in crate::managed) fn encode(
    budget: &SourceBudget,
    output: &Outgoing<SourceBudget>,
) -> Result<Source<SourceBudget>, Error> {
    if let Outgoing::Message(message) = output {
        let span = message.original();
        if span.range() == (0..span.source().text().len()) {
            return Ok(span.source().clone());
        }
        return Ok(Source::copy(span.text(), budget.clone())?);
    }
    let mut sink = sink(budget);
    match output {
        Outgoing::Message(message) => sink.literal(message.original().text().as_bytes())?,
        Outgoing::Cancel(id) => {
            sink.literal(
                br#"{"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"#,
            )?;
            sink.value(&Id(id))?;
            sink.literal(b"}}")?;
        }
        Outgoing::Responses { batch, replies } => {
            if *batch {
                sink.literal(b"[")?;
            }
            for (index, value) in replies.as_slice().iter().enumerate() {
                if index != 0 {
                    sink.literal(b",")?;
                }
                reply(&mut sink, value)?;
            }
            if *batch {
                sink.literal(b"]")?;
            }
        }
    }
    Ok(Source::from_buffer(sink.bytes)?)
}
