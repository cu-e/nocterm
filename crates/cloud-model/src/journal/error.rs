use crate::{
    identity::{EventSequence, IdentityError, JournalSequence},
    payload::PayloadError,
};
use nocterm_core::SharedCause;
use std::fmt;

#[derive(Clone, thiserror::Error)]
pub enum JournalError {
    #[error("the journal revision or immutable command content conflicts")]
    Conflict,
    #[error("the original journal writer is fenced")]
    Fenced,
    #[error("the requested journal was not found")]
    NotFound,
    #[error("the exact configured chat key material is unavailable")]
    KeyUnavailable,
    #[error("the journal is corrupt or its scoped record is invalid")]
    Corrupt,
    #[error("the native journal format requires explicit recovery")]
    UnsupportedFormat,
    #[error("journal sequence gap: expected {expected:?}, found {actual:?}")]
    Gap {
        expected: JournalSequence,
        actual: JournalSequence,
    },
    #[error("event replay gap: expected {expected:?}, found {actual:?}")]
    EventGap {
        expected: EventSequence,
        actual: EventSequence,
    },
    #[error("the journal batch or page exceeds its bounded capacity")]
    Capacity,
    #[error("the bounded journal authority read is busy; authority was not established")]
    Busy,
    #[error("the journal batch has inconsistent command, scope, checkpoint or outbox data")]
    InvalidBatch,
    #[error("a journal identity or counter is exhausted")]
    Identity(#[from] IdentityError),
    #[error("a journal payload is invalid")]
    Payload(#[from] PayloadError),
    #[error("the durable journal operation failed")]
    Storage(#[source] SharedCause),
}
impl fmt::Debug for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Conflict => "Conflict",
            Self::Fenced => "Fenced",
            Self::NotFound => "NotFound",
            Self::KeyUnavailable => "KeyUnavailable",
            Self::Corrupt => "Corrupt",
            Self::UnsupportedFormat => "UnsupportedFormat",
            Self::Gap { .. } => "Gap",
            Self::EventGap { .. } => "EventGap",
            Self::Capacity => "Capacity",
            Self::Busy => "Busy",
            Self::InvalidBatch => "InvalidBatch",
            Self::Identity(_) => "Identity",
            Self::Payload(_) => "Payload",
            Self::Storage(_) => "Storage",
        })
    }
}
