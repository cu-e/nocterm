use crate::{
    authority::StreamBinding,
    identity::{EventSequence, Fingerprint, WriterEpoch},
};

/// Exact original stream and writer incarnation. Capturing this DTO is not login.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionBinding {
    stream: StreamBinding,
    writer: WriterEpoch,
}
impl ProjectionBinding {
    pub fn new(stream: StreamBinding, writer: WriterEpoch) -> Self {
        Self { stream, writer }
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn writer(&self) -> WriterEpoch {
        self.writer
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectionCursor {
    sequence: EventSequence,
    head: Option<Fingerprint>,
}
impl ProjectionCursor {
    pub fn origin() -> Self {
        Self::default()
    }
    pub fn at(sequence: EventSequence, head: Fingerprint) -> Self {
        Self {
            sequence,
            head: Some(head),
        }
    }
    pub fn sequence(self) -> EventSequence {
        self.sequence
    }
    pub fn head(self) -> Option<Fingerprint> {
        self.head
    }
}
