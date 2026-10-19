use crate::DeviceUnlockError;
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("The vault is locked.")]
    Locked,
    #[error("The vault password is incorrect or the vault was modified.")]
    Authentication,
    #[error("The vault format is invalid or unsupported.")]
    Format,
    #[error("The vault changed in another process. Lock and unlock it again before saving.")]
    Conflict,
    #[error("Another vault operation is in progress. Try again.")]
    Busy,
    #[error("The vault already exists.")]
    AlreadyExists,
    #[error("The credential does not match this authentication request.")]
    Binding,
    #[error("The credential was not found.")]
    NotFound,
    #[error("The vault or credential exceeds the supported size limit.")]
    Limit,
    #[error("Use a master password of at least 8 characters and at most 4096 bytes.")]
    Password,
    #[error("Operating system randomness is unavailable.")]
    Randomness,
    #[error("The operation was cancelled because the vault was locked.")]
    Cancelled,
    #[error("Vault file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Device(#[from] DeviceUnlockError),
}
