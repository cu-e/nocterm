use crate::{DownloadRequest, Progress, UploadRequest};
use futures::future::BoxFuture;
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
    Runtime(#[source] Arc<dyn Error + Send + Sync>),
    #[error("Invalid transfer limit: {0}")]
    InvalidLimit(&'static str),
}
pub trait TransferService: Send + Sync + 'static {
    fn enqueue(&self, request: UploadRequest) -> Result<TransferId, TransferError>;
    fn enqueue_download(&self, request: DownloadRequest) -> Result<TransferId, TransferError>;
    fn snapshot(&self) -> Vec<Progress>;
    fn cancel(&self, id: TransferId) -> Result<(), TransferError>;
    fn retry(&self, id: TransferId, fs: Arc<dyn RemoteFs>) -> Result<TransferId, TransferError>;
    /// Closes admission and requests cancellation synchronously, before the future
    /// is first polled. The future retains cleanup resources independently of UI.
    fn freeze_and_drain(&self) -> BoxFuture<'static, Result<(), TransferError>>;
}
