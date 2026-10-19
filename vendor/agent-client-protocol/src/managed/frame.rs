//! Opaque validated JSON-RPC routing facts; typed payload parsing stays native.
use super::{Budget, Buffer, ManagedError, Span};

/// Original JSON-RPC request identifier. String values are decoded UTF-8 spans,
/// not JSON spelling: escaped and unescaped equal IDs must correlate equally.
pub enum MessageId<B: Budget> {
    /// The original JSON null ID.
    Null,
    /// The original signed integral JSON-RPC number.
    Number(i64),
    /// The actual decoded string, held by a prepaid source.
    String(Span<B>),
}
impl<B: Budget> Clone for MessageId<B> {
    fn clone(&self) -> Self {
        match self {
            Self::Null => Self::Null,
            Self::Number(id) => Self::Number(*id),
            Self::String(id) => Self::String(id.clone()),
        }
    }
}
impl<B: Budget> PartialEq for MessageId<B> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Number(left), Self::Number(right)) => left == right,
            (Self::String(left), Self::String(right)) => left.text() == right.text(),
            _ => false,
        }
    }
}
impl<B: Budget> Eq for MessageId<B> {}
impl<B: Budget> std::hash::Hash for MessageId<B> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        match self {
            Self::Null => 0u8.hash(state),
            Self::Number(id) => {
                1u8.hash(state);
                id.hash(state);
            }
            Self::String(id) => {
                2u8.hash(state);
                id.text().hash(state);
            }
        }
    }
}
/// Distinguishes protocol routing without deserializing method-specific data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageKind {
    /// An incoming call requires its original responder.
    Request,
    /// A notification has no response destination.
    Notification,
    /// A successful reply to one original outgoing request.
    Result,
    /// A remote JSON-RPC error, retained as original raw source.
    Error,
}
/// Opaque method body preserving absent versus supplied-null params.
pub enum Payload<B: Budget> {
    /// The source envelope omitted params entirely.
    Absent,
    /// The complete validated JSON value, including literal null.
    Supplied(Span<B>),
}
impl<B: Budget> Payload<B> {
    /// Borrow an original supplied raw JSON value.
    pub fn supplied(&self) -> Option<&Span<B>> {
        match self {
            Self::Absent => None,
            Self::Supplied(span) => Some(span),
        }
    }
}
/// A native-validated routing envelope, inseparable from its original wire source.
///
/// The native decoder validates JSON and the SDK's envelope grammar before
/// calling `validated`. This constructor checks routing shape only. It never
/// parses a JSON value or claims to authenticate external decoder assertions.
pub struct Envelope<B: Budget> {
    kind: MessageKind,
    id: Option<MessageId<B>>,
    method: Option<Span<B>>,
    payload: Payload<B>,
    original: Span<B>,
}
impl<B: Budget> Envelope<B> {
    /// Consume native-validated facts without allocating or cloning payload.
    pub fn validated(
        kind: MessageKind,
        id: Option<MessageId<B>>,
        method: Option<Span<B>>,
        payload: Payload<B>,
        original: Span<B>,
    ) -> Result<Self, ManagedError<B::Error>> {
        let valid = match kind {
            MessageKind::Request => id.is_some() && method.is_some(),
            MessageKind::Notification => id.is_none() && method.is_some(),
            MessageKind::Result | MessageKind::Error => {
                id.is_some() && method.is_none() && matches!(payload, Payload::Supplied(_))
            }
        };
        if !valid {
            return Err(ManagedError::InvalidEnvelope);
        }
        Ok(Self {
            kind,
            id,
            method,
            payload,
            original,
        })
    }
    pub(crate) fn belongs_to(&self, budget: &B) -> bool {
        self.original.belongs_to(budget)
            && self
                .method
                .as_ref()
                .is_none_or(|method| method.belongs_to(budget))
            && self.id.as_ref().is_none_or(|id| match id {
                MessageId::String(text) => text.belongs_to(budget),
                _ => true,
            })
            && self
                .payload
                .supplied()
                .is_none_or(|payload| payload.belongs_to(budget))
    }
    /// Original protocol routing kind.
    pub fn kind(&self) -> MessageKind {
        self.kind
    }
    /// Original decoded request ID, when present.
    pub fn id(&self) -> Option<&MessageId<B>> {
        self.id.as_ref()
    }
    /// Borrow the native-decoded method name.
    pub fn method(&self) -> Option<&str> {
        self.method.as_ref().map(Span::text)
    }
    pub(crate) fn method_span(&self) -> Option<Span<B>> {
        self.method.clone()
    }
    /// Borrow the actual guarded raw payload.
    pub fn payload(&self) -> &Payload<B> {
        &self.payload
    }
    /// Borrow original wire bytes; serialization does not reconstruct a DTO.
    pub fn original(&self) -> &Span<B> {
        &self.original
    }
}
/// An explicitly retained malformed member, preserving its original error shape.
pub struct Malformed<B: Budget> {
    original: Span<B>,
    response_only: bool,
    parse_error: bool,
}
impl<B: Budget> Malformed<B> {
    /// Retain original malformed source and SDK-compatible classification.
    pub fn validated(original: Span<B>, response_only: bool, parse_error: bool) -> Self {
        Self {
            original,
            response_only,
            parse_error,
        }
    }
    /// Original malformed wire bytes.
    pub fn original(&self) -> &Span<B> {
        &self.original
    }
    /// Whether original SDK handling ignores this response-shaped malformed value.
    pub fn response_only(&self) -> bool {
        self.response_only
    }
    /// Whether JSON lexical parsing failed rather than envelope validation.
    pub fn parse_error(&self) -> bool {
        self.parse_error
    }
}
/// One actual source-order batch position, including malformed members.
pub enum Entry<B: Budget> {
    /// A native-validated message at its original position.
    Message(Envelope<B>),
    /// An original malformed position; never silently omitted from a batch.
    Malformed(Malformed<B>),
}
/// Complete original transport frame; no SDK Value/typed decoding is performed.
pub enum Frame<B: Budget> {
    /// A single message and its original physical source.
    Single(Envelope<B>),
    /// A nonempty original-order collection of actual batch members.
    Batch(Buffer<Entry<B>, B>),
    /// A standalone malformed value, including an empty batch.
    Malformed(Malformed<B>),
}
impl<B: Budget> Frame<B> {
    pub(crate) fn belongs_to(&self, budget: &B) -> bool {
        fn entry<B: Budget>(entry: &Entry<B>, budget: &B) -> bool {
            match entry {
                Entry::Message(envelope) => envelope.belongs_to(budget),
                Entry::Malformed(value) => value.original.belongs_to(budget),
            }
        }
        match self {
            Self::Single(envelope) => envelope.belongs_to(budget),
            Self::Batch(entries) => {
                entries.belongs_to(budget)
                    && entries.as_slice().iter().all(|value| entry(value, budget))
            }
            Self::Malformed(value) => value.original.belongs_to(budget),
        }
    }
    /// Validate a nonempty batch without losing original source positions.
    pub fn batch(entries: Buffer<Entry<B>, B>) -> Result<Self, ManagedError<B::Error>> {
        if entries.is_empty() {
            return Err(ManagedError::InvalidEnvelope);
        }
        Ok(Self::Batch(entries))
    }
}
