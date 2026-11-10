use crate::{DownloadRequest, Progress, Request, UploadRequest};
use futures::future::BoxFuture;
use nocterm_core::SharedCause;
use nocterm_session::RemoteFs;
use std::{error::Error, fmt, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransferId([u8; 16]);
impl TransferId {
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self, TransferError> {
        if bytes == [0; 16] {
            Err(TransferError::InvalidId)
        } else {
            Ok(Self(bytes))
        }
    }
    pub fn bytes(self) -> [u8; 16] {
        self.0
    }
}
impl fmt::Display for TransferId {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                output.write_str("-")?;
            }
            write!(output, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl std::str::FromStr for TransferId {
    type Err = TransferError;
    /// The inverse of `Display`: 8-4-4-4-12 lowercase hexadecimal digits.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = text.as_bytes();
        if bytes.len() != 36 {
            return Err(TransferError::InvalidId);
        }
        let mut id = [0u8; 16];
        let mut index = 0;
        let mut position = 0;
        while position < bytes.len() {
            if matches!(position, 8 | 13 | 18 | 23) {
                if bytes[position] != b'-' {
                    return Err(TransferError::InvalidId);
                }
                position += 1;
                continue;
            }
            let high = hex_digit(bytes[position])?;
            let low = hex_digit(bytes[position + 1])?;
            id[index] = high << 4 | low;
            index += 1;
            position += 2;
        }
        Self::from_bytes(id)
    }
}
fn hex_digit(byte: u8) -> Result<u8, TransferError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(TransferError::InvalidId),
    }
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum TransferError {
    #[error("The transfer queue is full. Wait for a batch to finish.")]
    Full,
    #[error("No files were selected.")]
    Empty,
    #[error("Invalid transfer destination.")]
    InvalidDestination,
    #[error("The transfer no longer exists.")]
    Missing,
    #[error(
        "Selected paths exceed the transfer request limit. Select fewer files or their containing folder."
    )]
    RequestTooLarge,
    #[error("The transfer service is shutting down.")]
    ShuttingDown,
    #[error("Only a finished transfer may be retried.")]
    NotRetryable,
    #[error("An all-zero transfer ID is invalid.")]
    InvalidId,
    #[error("Transfer admission sequence exhausted.")]
    SequenceExhausted,
    #[error("Transfer runtime failed: {0}")]
    Runtime(#[source] SharedCause),
    #[error("Invalid transfer limit: {0}")]
    InvalidLimit(&'static str),
    #[error("Source and destination are both this computer.")]
    SameLocalEndpoints,
    #[error("The destination is inside a source folder.")]
    DestinationInsideSource,
}
impl TransferError {
    /// Preserve the original runtime error and its concrete source classification.
    pub fn runtime(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Runtime(SharedCause::new(error))
    }
}

pub trait TransferService: Send + Sync + 'static {
    fn enqueue(&self, request: UploadRequest) -> Result<TransferId, TransferError>;
    fn enqueue_download(&self, request: DownloadRequest) -> Result<TransferId, TransferError>;
    fn snapshot(&self) -> Vec<Progress>;
    /// A monotonic hint for coalesced snapshot readers. None keeps legacy adapters
    /// polling; a changed revision requires a fresh snapshot. It grants no authority.
    fn revision(&self) -> Option<u64> {
        None
    }
    /// Coalesced wakeup after a newer revision, including changes before first poll.
    /// None keeps adapters without notifications on the consumer's bounded cadence.
    fn changed_since(&self, _revision: u64) -> Option<BoxFuture<'static, ()>> {
        None
    }

    fn cancel(&self, id: TransferId) -> Result<(), TransferError>;
    fn retry(&self, id: TransferId, fs: Arc<dyn RemoteFs>) -> Result<TransferId, TransferError>;
    /// Closes admission and requests cancellation synchronously, before the future
    /// is first polled. The future retains cleanup resources independently of UI.
    fn freeze_and_drain(&self) -> BoxFuture<'static, Result<(), TransferError>>;
}

/// Starts and follows transfers for a consumer that owns no queue, retry or shutdown.
pub trait TransferRequests: Send + Sync + 'static {
    fn submit(&self, request: Request) -> Result<TransferId, TransferError>;
    /// `TransferError::Missing` once the id is unknown or evicted from history.
    fn progress(&self, id: TransferId) -> Result<Progress, TransferError>;
    fn cancel(&self, id: TransferId) -> Result<(), TransferError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_ids_round_trip_through_their_text() {
        let id = TransferId::from_bytes([
            0x8c, 0x1e, 0x00, 0x42, 0xab, 0xcd, 0x01, 0x23, 0x45, 0x67, 0x89, 0xef, 0x10, 0x20,
            0x30, 0x40,
        ])
        .unwrap();
        let text = id.to_string();
        assert_eq!(text, "8c1e0042-abcd-0123-4567-89ef10203040");
        assert_eq!(text.parse::<TransferId>().unwrap(), id);
    }

    #[test]
    fn malformed_and_zero_ids_are_invalid() {
        for text in [
            "",
            "00000000-0000-0000-0000-000000000000",
            "8C1E0042-ABCD-0123-4567-89EF10203040",
            "8c1e0042abcd0123456789ef1020304000",
            "8c1e0042-abcd-0123-4567-89ef1020304g",
            "8c1e0042-abcd-0123-4567-89ef102030",
            "8c1e0042_abcd-0123-4567-89ef10203040",
        ] {
            assert!(
                matches!(text.parse::<TransferId>(), Err(TransferError::InvalidId)),
                "{text:?}"
            );
        }
    }
}
