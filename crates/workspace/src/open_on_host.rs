//! Typed refusals when opening a program through a connected session.
use nocterm_session::{LaunchValidationError, Target};

/// Why a program could not be opened through a connected session.
#[derive(Debug, thiserror::Error)]
pub enum OpenOnHostError {
    #[error("Use the admitted project terminal provider in this context.")]
    ProtectedContext,
    #[error("No connected session to {target} is open.")]
    NoConnectedSession { target: Target },
    #[error("{0}")]
    InvalidLaunch(#[from] LaunchValidationError),
}
