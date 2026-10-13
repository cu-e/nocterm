//! Refusals for literal Explorer path insertion.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FileDropError {
    #[error("Connect and finish authentication before dropping paths.")]
    NotConnected,
    #[error("This path cannot be represented as terminal text.")]
    NonUtf8Path,
    #[error("No file paths were selected.")]
    EmptySelection,
    #[error("Path insertion needs a supported POSIX shell or PowerShell.")]
    UnsupportedShell,
    #[error("Paths containing control characters cannot be inserted safely.")]
    ControlCharacters,
    #[error("The selected paths exceed 256 KiB; select fewer items.")]
    TooLarge,
    #[error("The selected paths cannot be inserted safely.")]
    UnsafeSelection,
    #[error("{0}")]
    Terminal(#[from] nocterm_workspace::TerminalError),
}
