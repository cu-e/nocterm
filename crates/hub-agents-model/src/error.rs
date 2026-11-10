//! Typed failures of the agent ports and of the forms people fill in.

use nocterm_hubs_model::HubError;

/// Why a form cannot be saved as typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    #[error("Enter a name.")]
    NameMissing,
    #[error("The name is too long.")]
    NameTooLong,
    #[error("The instructions are too long.")]
    InstructionsTooLong,
    #[error("Choose a provider.")]
    ProviderMissing,
    #[error("Add at least one model.")]
    ModelMissing,
    #[error("A model name is too long.")]
    ModelTooLong,
    #[error("The address must start with https:// or http://.")]
    BaseUrl,
}

/// Why an agent, provider or server call failed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum HubAgentsError {
    /// The hub could not be reached or refused the call; the cause tells why.
    #[error(transparent)]
    Hub(#[from] HubError),
    /// The form breaks a rule; nothing was sent.
    #[error(transparent)]
    Draft(#[from] DraftError),
    /// The hub answered with a record this client cannot read.
    #[error("the hub sent a record this client cannot read")]
    Malformed,
    /// A limit set by the hub's administrator is reached.
    #[error("a limit of the hub is reached")]
    LimitReached,
}
