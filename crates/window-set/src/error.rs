//! Why the set of open windows could not be installed.

use nocterm_store::StoreError;

/// The set could not be kept for the next start.
#[derive(Debug, thiserror::Error)]
pub enum WindowSetError {
    /// Shutdown capture was closed already, so a final save could not be promised.
    #[error("the open windows cannot be saved at shutdown")]
    Registration(#[source] StoreError),
}
