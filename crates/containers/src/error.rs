//! Typed failures of engine output and command execution.
use std::fmt;

use nocterm_session::ExecError;

/// The kind of object whose required identifier was missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingEntity {
    Container,
    Image,
}

impl fmt::Display for ListingEntity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Container => "container",
            Self::Image => "image",
        })
    }
}

/// A successful command returned an unreadable listing.
#[derive(Debug, thiserror::Error)]
pub enum ListingError {
    /// One-based line in the complete listing; the native source retains its column.
    #[error("the engine's listing could not be read at line {line}: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    /// Zero-based position in the listing.
    #[error("the engine's listing object {index} is not a JSON object")]
    ExpectedObject { index: usize },
    /// Zero-based position in the listing.
    #[error("the engine listed a {entity} without an id (object {index})")]
    MissingId { entity: ListingEntity, index: usize },
}

/// Why the containers of a host are not known.
#[derive(Debug, thiserror::Error)]
pub enum ContainersError {
    #[error("Neither Docker nor Podman is installed.")]
    NotInstalled,
    #[error("The session is not connected.")]
    Disconnected,
    #[error("This host cannot run programs.")]
    Unsupported,
    #[error("{0}")]
    Parse(#[from] ListingError),
    #[error("{0}")]
    Execution(#[source] ExecError),
    /// The bounded stderr tail is output data, not a flattened error source.
    #[error("{}", exit_message(.status, .stderr_tail))]
    Exit {
        status: Option<u32>,
        stderr_tail: String,
    },
}

impl From<ExecError> for ContainersError {
    fn from(error: ExecError) -> Self {
        match error {
            ExecError::NotFound(_) => Self::NotInstalled,
            ExecError::Disconnected => Self::Disconnected,
            ExecError::Unsupported => Self::Unsupported,
            error @ ExecError::Failed(_) => Self::Execution(error),
        }
    }
}

fn exit_message(status: &Option<u32>, stderr_tail: &str) -> String {
    let said = stderr_tail
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty());
    match (said, status) {
        (Some(line), _) => line.to_owned(),
        (None, Some(status)) => format!("the program exited with status {status}"),
        (None, None) => "the program was stopped".into(),
    }
}
