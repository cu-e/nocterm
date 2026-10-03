//! Explicit launch options, independent of a transport or shell parser.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    pub fn validate(&self) -> Result<(), String> {
        if self
            .program
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.contains('\0'))
        {
            return Err("Shell executable must be nonempty and contain no NUL.".into());
        }
        if self
            .args
            .iter()
            .chain(self.cwd.iter())
            .chain(self.env.values())
            .any(|s| s.contains('\0'))
        {
            return Err("Shell launch values must contain no NUL.".into());
        }
        if self.env.keys().any(|name| {
            name.is_empty()
                || !name.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
        }) {
            return Err("Environment names must be shell identifiers.".into());
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
    fn paths_are_single_literal_arguments() {
        assert_eq!(quote_posix("a'b $(id)"), "'a'\\''b $(id)'");
        assert_eq!(quote_powershell("a'b"), "'a''b'");
    }
}
