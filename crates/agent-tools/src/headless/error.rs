use nocterm_core::{SharedCause, SharedError};
use nocterm_transfers::{OwnedTransferError, TransferError};

#[derive(Clone, Debug, thiserror::Error)]
pub enum HeadlessError {
    #[error("the native session failed: {0}")]
    Session(#[source] nocterm_session::SessionError),
    #[error("the headless session registry is closed")]
    Closed,
    #[error("the headless session capacity is full")]
    Capacity,
    #[error("the original headless session is stale")]
    StaleSession,
    #[error("the headless session identity is already registered")]
    IdentityConflict,
    #[error("the original headless session is not connected")]
    NotConnected,
    #[error("the target requires explicit authentication")]
    NeedsUnlock,
    #[error("this headless session has no structured execution capability")]
    ExecutionUnsupported,
    #[error("the headless terminal geometry exceeds its allocation limits")]
    InvalidGeometry,
    #[error("the agent input exceeds sixteen KiB")]
    InputTooLarge,
    #[error("the session input queue refused the submission")]
    InputRefused,
    #[error("a known empty shell prompt is required")]
    CommandPromptUnavailable,
    #[error("another live command owns this terminal")]
    LiveCommandOwned,
    #[error("the text cannot be represented by the original session charset")]
    Encoding(#[from] nocterm_session::TextEncodingError),
    #[error("the original cloud execution authority does not match")]
    Authority(#[from] nocterm_cloud_model::authority::AuthorityError),
    #[error("this headless session has no file system capability")]
    FilesUnsupported,
    #[error("this headless run has no file transfer capability")]
    TransfersUnavailable,
    #[error("the file transfer was refused or failed")]
    Transfer(#[source] SharedError<OwnedTransferError>),
}
impl From<OwnedTransferError> for HeadlessError {
    fn from(error: OwnedTransferError) -> Self {
        Self::Transfer(SharedError::from_error(error))
    }
}
impl From<TransferError> for HeadlessError {
    fn from(error: TransferError) -> Self {
        OwnedTransferError::from(error).into()
    }
}
impl From<HeadlessError> for nocterm_cloud_model::tool::ToolExecutionError {
    fn from(error: HeadlessError) -> Self {
        use nocterm_cloud_model::tool::ToolExecutionError;
        match error {
            HeadlessError::StaleSession | HeadlessError::NotConnected | HeadlessError::Closed => {
                ToolExecutionError::StaleSession
            }
            HeadlessError::Session(error) => ToolExecutionError::Execution(SharedCause::new(error)),
            HeadlessError::Capacity => ToolExecutionError::Capacity,
            HeadlessError::NeedsUnlock => ToolExecutionError::NeedsUnlock,
            HeadlessError::ExecutionUnsupported
            | HeadlessError::FilesUnsupported
            | HeadlessError::TransfersUnavailable => ToolExecutionError::UnsupportedCapability,
            HeadlessError::Transfer(error)
                if matches!(error.as_ref(), OwnedTransferError::TooManyActive { .. }) =>
            {
                ToolExecutionError::Capacity
            }
            HeadlessError::Authority(error) => error.into(),
            error => ToolExecutionError::Execution(SharedCause::new(error)),
        }
    }
}
