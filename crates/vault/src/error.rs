use crate::DeviceUnlockError;
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("The captured account is no longer current.")]
    Stale,
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
    #[error("The vault is not reachable right now. Try again when you are online.")]
    Offline,
    #[error("This vault does not support that operation.")]
    Unsupported,
    #[error("Vault file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Device(#[from] DeviceUnlockError),
}

#[cfg(test)]
mod tests {
    use super::VaultError;

    #[test]
    fn offline_and_unsupported_explain_themselves_to_the_user() {
        assert!(VaultError::Offline.to_string().contains("online"));
        assert!(VaultError::Unsupported.to_string().contains("not support"));
    }
}
