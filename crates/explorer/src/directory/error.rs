//! Failures at the shared read-only directory boundary.
#[derive(Clone, Debug, thiserror::Error)]
pub enum DirectoryError {
    #[error("{0}")]
    Path(
        #[from]
        #[source]
        super::PathError,
    ),
    #[error("Reconnect to load directory suggestions.")]
    Disconnected,
    #[error("The home directory is unavailable.")]
    HomeUnavailable,
    #[error("This directory belongs to a different filesystem source.")]
    SourceMismatch,
    #[error("Directory exceeds the listing limit; enter a narrower path.")]
    ListingLimit,
    #[error("{0}")]
    Local(
        #[from]
        #[source]
        nocterm_session::local_fs::LocalFsError,
    ),
    #[error("{0}")]
    Remote(
        #[from]
        #[source]
        nocterm_session::FsError,
    ),
}
