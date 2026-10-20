//! Agent settings rules without input widgets or native environment lookup.
use crate::{AgentServerSettings, agents::AgentCatalog};
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentValidationError {
    #[error("Agent ids use letters, numbers, underscore or dash, up to 64 characters.")]
    InvalidId,
    #[error("{0}: enter an executable.")]
    MissingCommand(String),
    #[error(
        "{0}: inherit credential variables from your environment instead of storing them in settings."
    )]
    CredentialOverride(String),
    #[error(
        "{0}: environment names must be identifiers; SSH, Nocterm and process injection variables are forbidden."
    )]
    InvalidEnv(String),
    #[error("{0}: values cannot contain NUL.")]
    ContainsNul(String),
}

/// Checks one agent's settings; only `catalog` agents may leave the command unset.
pub fn validate_agent(
    catalog: &AgentCatalog,
    id: &str,
    setting: &AgentServerSettings,
) -> Result<(), AgentValidationError> {
    if !crate::agents::valid_id(id) {
        return Err(AgentValidationError::InvalidId);
    }
    if setting.enabled && setting.command.is_none() && !catalog.is_builtin(id) {
        return Err(AgentValidationError::MissingCommand(id.to_owned()));
    }
    if setting
        .env
        .keys()
        .any(|key| crate::env::credential_override(key))
    {
        return Err(AgentValidationError::CredentialOverride(id.to_owned()));
    }
    if setting
        .env
        .keys()
        .chain(setting.inherit_env.iter())
        .any(|key| !crate::env::valid_name(key) || crate::env::blocked(key))
    {
        return Err(AgentValidationError::InvalidEnv(id.to_owned()));
    }
    if setting
        .command
        .iter()
        .chain(setting.name.iter())
        .chain(setting.args.iter().flatten())
        .chain(setting.env.values())
        .any(|v| v.contains('\0'))
    {
        return Err(AgentValidationError::ContainsNul(id.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_custom_and_prohibited_environment() {
        let catalog = AgentCatalog::default();
        let mut agent = AgentServerSettings::default();
        assert!(validate_agent(&catalog, "custom", &agent).is_err());
        agent.command = Some("/opt/acp".into());
        assert!(validate_agent(&catalog, "custom", &agent).is_ok());
        agent.env.insert("OPENAI_API_KEY".into(), "marker".into());
        assert!(validate_agent(&catalog, "custom", &agent).is_err());
        agent.env.clear();
        agent.inherit_env.push("SSH_AUTH_SOCK".into());
        assert!(validate_agent(&catalog, "custom", &agent).is_err());
        agent.inherit_env.clear();
        agent.args = Some(vec!["nul\0".into()]);
        assert!(validate_agent(&catalog, "custom", &agent).is_err());
    }
}
