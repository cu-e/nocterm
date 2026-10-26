use serde::{Deserialize, Serialize};

use crate::ProjectValidationError;

/// A literal normalized absolute POSIX path, never a path on the client OS.
///
/// This performs structural validation only. Ordinary sessions apply the host's
/// usual filesystem permissions when opening the supplied working directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RootPath(String);

impl RootPath {
    pub fn parse(value: impl Into<String>) -> Result<Self, ProjectValidationError> {
        let value = value.into();
        if !value.starts_with('/')
            || value.len() > 4096
            || value.chars().any(char::is_control)
            || value != "/"
                && value[1..]
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(ProjectValidationError::InvalidRoot);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RootPath {
    type Error = ProjectValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RootPath> for String {
    fn from(value: RootPath) -> Self {
        value.0
    }
}
