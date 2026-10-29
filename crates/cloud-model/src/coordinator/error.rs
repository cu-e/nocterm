use crate::{
    engine::EngineError, inference::InferenceError, journal::JournalError, tool::ToolExecutionError,
};
use nocterm_core::SharedCause;
use std::fmt;

#[derive(Clone, thiserror::Error)]
pub enum CoordinatorError {
    #[error("the identity counter is invalid or exhausted")]
    Identity(#[from] crate::identity::IdentityError),
    #[error("the run transition was refused")]
    Run(#[from] crate::run::RunError),
    #[error("the tool grant was refused")]
    Grant(#[from] crate::grant::GrantError),
    #[error("the bounded payload was refused")]
    Payload(#[from] crate::payload::PayloadError),
    #[error("the original command source is no longer current")]
    StaleSource,
    #[error("the command coordinates or semantic body are invalid")]
    InvalidCommand,
    #[error("the original command or control revision conflicts")]
    Conflict,
    #[error("the native receive allocation was refused")]
    Intake(#[from] super::IntakeError),
    #[error("the bounded command or run capacity is full")]
    Overloaded,
    #[error("the command or grant is not currently within its deadline")]
    Expired,
    #[error("the original run is stopped or interrupted")]
    Stopped,
    #[error("the task needs an explicit finite grant")]
    NeedsGrant,
    #[error("the task needs a new explicit authentication")]
    NeedsUnlock,
    #[error("the task needs explicit provider access")]
    NeedsProviderAccess,
    #[error("this requested capability is not supported")]
    Unsupported,
    #[error("a supervised native task failed; fresh effects are paused")]
    NativeTaskFailed,
    #[error("the durable journal refused the operation")]
    Journal(#[from] JournalError),
    #[error("the model operation failed")]
    Engine(#[from] EngineError),
    #[error("the original tool operation failed")]
    Tool(#[from] ToolExecutionError),
    #[error("the finite inference budget refused the operation")]
    Inference(#[from] InferenceError),
    #[error("the native coordinator collaborator failed")]
    Backend(#[source] SharedCause),
}
impl CoordinatorError {
    pub fn is_needs_material(&self) -> bool {
        matches!(self, Self::Journal(JournalError::KeyUnavailable))
    }
}
impl fmt::Debug for CoordinatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Identity(_) => "Identity",
            Self::Run(_) => "Run",
            Self::Grant(_) => "Grant",
            Self::Payload(_) => "Payload",
            Self::StaleSource => "StaleSource",
            Self::InvalidCommand => "InvalidCommand",
            Self::Conflict => "Conflict",
            Self::Overloaded => "Overloaded",
            Self::Intake(_) => "Intake",
            Self::Expired => "Expired",
            Self::Stopped => "Stopped",
            Self::NeedsGrant => "NeedsGrant",
            Self::NeedsUnlock => "NeedsUnlock",
            Self::NeedsProviderAccess => "NeedsProviderAccess",
            Self::Unsupported => "Unsupported",
            Self::NativeTaskFailed => "NativeTaskFailed",
            Self::Journal(_) => "Journal",
            Self::Engine(_) => "Engine",
            Self::Tool(_) => "Tool",
            Self::Inference(_) => "Inference",
            Self::Backend(_) => "Backend",
        })
    }
}
