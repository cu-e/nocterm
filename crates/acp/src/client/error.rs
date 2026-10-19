//! Typed startup failures, converted to the public `AgentError` vocabulary at the port.
use nocterm_ai::AgentError;

use crate::process::CleanupError;

/// Why an agent subprocess could not be brought to a usable protocol session.
#[derive(Debug, thiserror::Error)]
pub(super) enum StartupError {
    #[error("Agent startup cancelled")]
    Cancelled,
    #[error("Agent {0} unavailable")]
    MissingPipe(&'static str),
    #[error("{0}")]
    ThreadSpawn(#[source] std::io::Error),
    #[error("Agent initialization timed out")]
    InitializationTimeout,
    #[error("Unsupported ACP protocol version")]
    UnsupportedProtocol,
    #[error("Agent exited during initialization")]
    ExitedDuringInitialization,
    #[error("Agent process cleanup could not be confirmed: {0}")]
    Cleanup(#[source] CleanupError),
    #[error(transparent)]
    Agent(#[from] AgentError),
}

impl From<StartupError> for AgentError {
    fn from(error: StartupError) -> Self {
        match error {
            StartupError::Agent(error) => error,
            StartupError::Cleanup(error) => Self::CleanupUnconfirmed(error.to_string()),
            error => Self::Io(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_failures_keep_their_public_text() {
        for (error, text) in [
            (StartupError::Cancelled, "Agent startup cancelled"),
            (
                StartupError::MissingPipe("stdin"),
                "Agent stdin unavailable",
            ),
            (
                StartupError::InitializationTimeout,
                "Agent initialization timed out",
            ),
            (
                StartupError::UnsupportedProtocol,
                "Unsupported ACP protocol version",
            ),
            (
                StartupError::ExitedDuringInitialization,
                "Agent exited during initialization",
            ),
        ] {
            assert!(matches!(AgentError::from(error), AgentError::Io(message) if message == text));
        }
    }

    #[test]
    fn remote_failures_pass_through_unchanged() {
        let error = AgentError::AuthRequired("login".into());
        assert!(matches!(
            AgentError::from(StartupError::from(error)),
            AgentError::AuthRequired(message) if message == "login"
        ));
    }
}
