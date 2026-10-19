//! Exact pinned SDK framing behavior over actual original Source capabilities.
mod authentication;
mod channel;
mod content;
mod driver;
mod events;
mod literals;
mod models;
mod notices;
mod outgoing;
mod permission;
mod redaction;
mod results;
mod stderr;
mod transport;
use super::{budget::SourceBudget, wire};
use agent_client_protocol::managed::{Entry, Envelope, Frame, MessageId, MessageKind, Source};
use agent_client_protocol::{RawJsonRpcMessage, TransportBatchEntry, TransportFrame};
use nocterm_ai::{
    history::source::{SourceLease, SourceProducer},
    model::retention::{RetentionAuthority, document::DocumentRetention},
};
use serde_json::Value;

pub(super) struct Fixture {
    pub(super) budget: SourceBudget,
    _lease: SourceLease,
    _producer: SourceProducer,
}
pub(super) fn fixture() -> Fixture {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let producer = SourceProducer::new(roles.take_source().unwrap()).unwrap();
    let lease = producer.capture().unwrap();
    let budget = SourceBudget::new(lease.context().unwrap());
    Fixture {
        budget,
        _lease: lease,
        _producer: producer,
    }
}
#[derive(Debug, PartialEq)]
enum Signature {
    Message {
        kind: MessageKind,
        id: Option<Value>,
        method: Option<String>,
        payload: Option<Value>,
    },
    Malformed {
        parse: bool,
        response_only: bool,
    },
    Batch(Vec<Signature>),
}
fn response_only(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        !object.contains_key("method")
            && (object.contains_key("result") || object.contains_key("error"))
    })
}
fn original_message(message: RawJsonRpcMessage) -> Signature {
    use agent_client_protocol::schema::v1::Response;
    match message {
        RawJsonRpcMessage::Request(request) => Signature::Message {
            kind: MessageKind::Request,
            id: Some(serde_json::to_value(request.id).unwrap()),
            method: Some(request.method.to_string()),
            payload: request.params.map(|value| value.into_value()),
        },
        RawJsonRpcMessage::Notification(notification) => Signature::Message {
            kind: MessageKind::Notification,
            id: None,
            method: Some(notification.method.to_string()),
            payload: notification.params.map(|value| value.into_value()),
        },
        RawJsonRpcMessage::Response(Response::Result { id, result }) => Signature::Message {
            kind: MessageKind::Result,
            id: Some(serde_json::to_value(id).unwrap()),
            method: None,
            payload: Some(result),
        },
        RawJsonRpcMessage::Response(Response::Error { id, error }) => Signature::Message {
            kind: MessageKind::Error,
            id: Some(serde_json::to_value(id).unwrap()),
            method: None,
            payload: Some(serde_json::to_value(error).unwrap()),
        },
    }
}
fn original(raw: &str) -> Signature {
    match TransportFrame::parse_json(raw) {
        TransportFrame::Single(message) => original_message(message),
        TransportFrame::Malformed { raw, error } => Signature::Malformed {
            parse: i32::from(error.code) == -32700,
            response_only: serde_json::from_str(&raw).is_ok_and(|value| response_only(&value)),
        },
        TransportFrame::Batch(batch) => Signature::Batch(
            batch
                .into_entries()
                .map(|entry| match entry {
                    TransportBatchEntry::Message(message) => original_message(message),
                    TransportBatchEntry::Malformed { raw, .. } => Signature::Malformed {
                        parse: false,
                        response_only: response_only(&raw),
                    },
                })
                .collect(),
        ),
    }
}
fn native_message(envelope: Envelope<SourceBudget>) -> Signature {
    let id = envelope.id().map(|id| match id {
        MessageId::Null => Value::Null,
        MessageId::Number(id) => Value::from(*id),
        MessageId::String(text) => Value::String(text.text().into()),
    });
    let mut payload = envelope
        .payload()
        .supplied()
        .map(|span| serde_json::from_str::<Value>(span.text()).unwrap());
    if envelope.kind() == MessageKind::Error {
        // The raw source remains untouched; compare the SDK's retained error
        // model, which omits unknown fields and normalizes optional null data.
        let error = serde_json::from_value::<agent_client_protocol::Error>(payload.take().unwrap())
            .unwrap();
        payload = Some(serde_json::to_value(error).unwrap());
    }
    if matches!(
        envelope.kind(),
        MessageKind::Request | MessageKind::Notification
    ) && payload == Some(Value::Null)
    {
        payload = None;
    }
    Signature::Message {
        kind: envelope.kind(),
        id,
        method: envelope.method().map(str::to_owned),
        payload,
    }
}
fn native(raw: &str) -> Signature {
    let fixture = fixture();
    let source = Source::copy(raw, fixture.budget.clone()).unwrap();
    match wire::frame(&source, fixture.budget).unwrap() {
        Frame::Single(envelope) => native_message(envelope),
        Frame::Malformed(malformed) => Signature::Malformed {
            parse: malformed.parse_error(),
            response_only: malformed.response_only(),
        },
        Frame::Batch(batch) => Signature::Batch(
            batch
                .into_values()
                .map(|entry| match entry {
                    Entry::Message(envelope) => native_message(envelope),
                    Entry::Malformed(malformed) => Signature::Malformed {
                        parse: malformed.parse_error(),
                        response_only: malformed.response_only(),
                    },
                })
                .collect(),
        ),
    }
}
#[test]
fn controlled_frames_match_pinned_sdk_envelopes_and_duplicate_winners() {
    for raw in [
        r#"{"jsonrpc":"2.0","method":"m"}"#,
        r#"{"jsonrpc":"2.0","method":"m","id":null,"params":null}"#,
        r#"{"jsonrpc":"2.0","method":"m","id":1,"params":[1,"x"]}"#,
        r#"{"jsonrpc":"2.0","method":"m","id":-0,"params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"m","id":9223372036854775808}"#,
        r#"{"jsonrpc":"2.0","method":"m","id":"a\u0062","params":{}}"#,
        r#"{"jsonrpc":"1.0","method":true,"id":false,"jsonrpc":"2.0","method":"m","id":3}"#,
        r#"{"jsonrpc":[2.0],"method":"m"}"#,
        r#"{"jsonrpc":"2.0","method":"m","params":false}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":0,"error":{}}"#,
        r#"{"jsonrpc":"2.0","result":42}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"x","data":{"future":true}}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":1.0,"message":"x"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-0,"message":"x"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":null}}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":1,"message":"x","future":5}}"#,
        r#"[{"jsonrpc":"2.0","method":"m","id":1},null,[],{"result":42}]"#,
        r#"[]"#,
        r#"true"#,
        r#"{}"#,
        r#"{"method":"m"}"#,
        r#"{"jsonrpc":"2.0","method":"\uD83D\uDE00"}"#,
        r#"{"jsonrpc":"2.0","method":"\u+000"}"#,
        r#"{"jsonrpc":"2.0","method":"\uD800"}"#,
        r#"{"jsonrpc":"2.0","method":"m","future":1e9999}"#,
        r#"{"jsonrpc":"2.0","method":"m","future":[1,]}"#,
    ] {
        assert_eq!(native(raw), original(raw), "wire: {raw}");
    }
}
#[test]
fn original_sdk_outer_recursion_boundary_is_preserved_without_history_policy() {
    for depth in [124, 125, 126, 127, 128, 129, 200] {
        let raw = format!(
            r#"{{"jsonrpc":"2.0","method":"m","params":{}0{}}}"#,
            "[".repeat(depth),
            "]".repeat(depth)
        );
        assert_eq!(
            native(&raw),
            original(&raw),
            "actual pinned SDK nesting {depth}"
        );
    }
}
