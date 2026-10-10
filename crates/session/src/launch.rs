//! Explicit launch options, independent of a transport or shell parser.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An invalid literal launch value. Errors never retain environment values.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LaunchValidationError {
    #[error("Shell executable must be nonempty and contain no NUL.")]
    InvalidProgram,
    #[error("Shell launch values must contain no NUL.")]
    NulArgument { index: usize },
    #[error("Shell launch values must contain no NUL.")]
    NulDirectory,
    #[error("Shell launch values must contain no NUL.")]
    NulEnvironmentValue { key: String },
    #[error("Environment names must be shell identifiers.")]
    InvalidEnvironmentName,
}

/// An executable and its arguments. Empty remote options preserve SSH's shell request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ShellLaunch {
    /// Executable; absent means the operating system or server default shell.
    pub program: Option<String>,
    /// Arguments, passed individually rather than parsed as a command string.
    pub args: Vec<String>,
    /// Initial directory; absent uses the user's home directory.
    pub cwd: Option<String>,
    /// Additional environment variables. Do not store secrets here.
    pub env: BTreeMap<String, String>,
    /// Enable ephemeral cwd and prompt integration for supported shells.
    pub integration: bool,
}

impl Default for ShellLaunch {
    fn default() -> Self {
        Self {
            program: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            integration: true,
        }
    }
}

impl ShellLaunch {
    /// Reject values that cannot be passed safely through process or SSH APIs.
    pub fn validate(&self) -> Result<(), LaunchValidationError> {
        if self
            .program
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.contains('\0'))
        {
            return Err(LaunchValidationError::InvalidProgram);
        }
        if let Some(index) = self.args.iter().position(|value| value.contains('\0')) {
            return Err(LaunchValidationError::NulArgument { index });
        }
        if self.cwd.as_ref().is_some_and(|value| value.contains('\0')) {
            return Err(LaunchValidationError::NulDirectory);
        }
        if let Some((key, _)) = self.env.iter().find(|(_, value)| value.contains('\0')) {
            return Err(LaunchValidationError::NulEnvironmentValue { key: key.clone() });
        }
        if self.env.keys().any(|name| {
            name.is_empty()
                || !name.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
        }) {
            return Err(LaunchValidationError::InvalidEnvironmentName);
        }
        Ok(())
    }
}

/// Quotes a single argument for a POSIX shell without expanding its contents.
pub fn quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Quotes a single argument for PowerShell.
pub fn quote_powershell(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_environment_identifiers_and_nul() {
        let mut launch = ShellLaunch::default();
        launch.env.insert("BAD;NAME".into(), "x".into());
        assert!(launch.validate().is_err());
        launch.env.clear();
        launch.args.push("a\0b".into());
        assert!(launch.validate().is_err());
    }
    #[test]
    fn launch_errors_locate_nul_without_retaining_secret_values() {
        let mut launch = ShellLaunch {
            args: vec!["literal".into(), "a\0b".into()],
            ..Default::default()
        };
        assert_eq!(
            launch.validate(),
            Err(LaunchValidationError::NulArgument { index: 1 })
        );
        launch.args.clear();
        launch.cwd = Some("a\0b".into());
        assert_eq!(launch.validate(), Err(LaunchValidationError::NulDirectory));
        launch.cwd = None;
        launch
            .env
            .insert("AUTH_TOKEN".into(), "private-value\0tail".into());
        let error = launch.validate().unwrap_err();
        assert_eq!(
            error,
            LaunchValidationError::NulEnvironmentValue {
                key: "AUTH_TOKEN".into()
            }
        );
        assert_eq!(
            error.to_string(),
            "Shell launch values must contain no NUL."
        );
        assert!(!format!("{error:?}").contains("private-value"));
        launch.program = Some(String::new());
        assert_eq!(
            launch.validate(),
            Err(LaunchValidationError::InvalidProgram)
        );
    }

    #[test]
    fn validation_checks_literals_without_shell_expansion_or_filesystem_access() {
        let mut launch = ShellLaunch {
            program: Some("  literal program  ".into()),
            args: vec!["$(id)".into(), "a'b".into(), "".into()],
            cwd: Some("/a/nonexistent/session/directory".into()),
            env: [("_SESSION1".into(), "private $(command) value".into())].into(),
            ..Default::default()
        };
        assert!(launch.validate().is_ok());
        assert_eq!(launch.program.as_deref(), Some("  literal program  "));
        for key in ["", "1NAME", "BAD;NAME", "a-b", "tést"] {
            launch.env = [(key.into(), "private-value".into())].into();
            assert_eq!(
                launch.validate(),
                Err(LaunchValidationError::InvalidEnvironmentName)
            );
        }
    }

    #[test]
    fn paths_are_single_literal_arguments() {
        assert_eq!(quote_posix("a'b $(id)"), "'a'\\''b $(id)'");
        assert_eq!(quote_powershell("a'b"), "'a''b'");
    }
}
