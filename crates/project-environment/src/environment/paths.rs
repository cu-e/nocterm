use nocterm_project_providers::{EnvironmentError, LanguageSelection, ProjectSetupError};
pub(crate) struct InstallPaths {
    home: String,
}
impl InstallPaths {
    pub(crate) fn new(home: &str) -> Result<Self, EnvironmentError> {
        if !home.starts_with('/')
            || home.len() > 4096
            || home.chars().any(char::is_control)
            || home.split('/').any(|part| matches!(part, "." | ".."))
        {
            return Err(EnvironmentError::ReadOnlyHome);
        }
        Ok(Self {
            home: home.to_owned(),
        })
    }
    pub(crate) fn home(&self) -> &str {
        &self.home
    }
}
pub(super) fn installation_error(
    error: ProjectSetupError,
    selection: &LanguageSelection,
) -> EnvironmentError {
    match error {
        ProjectSetupError::CommandFailed {
            status: Some(79), ..
        } => EnvironmentError::ReadOnlyHome,
        ProjectSetupError::CommandFailed {
            status: Some(78), ..
        } => EnvironmentError::UnsupportedVersion {
            language: selection.language(),
            reason: "the requested official artifact is unavailable for this platform",
        },
        ProjectSetupError::CommandFailed {
            status: Some(77), ..
        } => EnvironmentError::PrivilegeRequired,
        error => error.into(),
    }
}
