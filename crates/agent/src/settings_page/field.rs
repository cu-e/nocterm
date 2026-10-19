//! Feature-specific settings syntax and validation errors.

/// Why text typed into a field cannot be saved; `Display` is the message
/// shown beside the field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FieldError {
    #[error("Enter an absolute path, or leave empty for the private workspace.")]
    InvalidWorkingDirectory,
    #[error("Inherited variables: enter a JSON array of names.")]
    InheritedNotNames,
    #[error("An agent named `{0}` already exists.")]
    DuplicateAgent(String),
    #[error(transparent)]
    Agent(#[from] nocterm_ai::AgentValidationError),
    #[error(transparent)]
    Input(#[from] nocterm_ui::settings_input::SettingsInputError),
}

pub(super) use nocterm_ui::settings_input::{environment, json, optional, string_list};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn agent_field_errors_keep_their_messages() {
        assert_eq!(
            FieldError::DuplicateAgent("codex".into()).to_string(),
            "An agent named `codex` already exists."
        );
        assert_eq!(
            FieldError::from(nocterm_ai::AgentValidationError::InvalidId).to_string(),
            "Agent ids use letters, numbers, underscore or dash, up to 64 characters."
        );
    }
}
