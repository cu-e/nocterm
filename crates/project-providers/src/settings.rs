//! Defaults for built-in container workspace creation, without filesystem access.
use nocterm_project_model::RootPath;
use nocterm_settings::SettingsSection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Defaults applied to new workspace drafts; saved locations remain unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspacesSettings {
    /// Absolute POSIX project directory used when a container directory is left empty.
    pub container_directory: String,
}

impl Default for WorkspacesSettings {
    fn default() -> Self {
        Self {
            container_directory: "/home".into(),
        }
    }
}

impl SettingsSection for WorkspacesSettings {
    const KEY: &'static str = "workspaces";

    fn sanitize(&mut self) {
        if RootPath::parse(self.container_directory.clone()).is_err() {
            self.container_directory = Self::default().container_directory;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_workspace_defaults_use_home_without_changing_valid_literals() {
        assert_eq!(WorkspacesSettings::default().container_directory, "/home");
        let mut custom = WorkspacesSettings {
            container_directory: "/home/team project".into(),
        };
        custom.sanitize();
        assert_eq!(custom.container_directory, "/home/team project");
    }

    #[test]
    fn malformed_loaded_directory_defaults_are_replaced_with_home() {
        for invalid in [
            "",
            "relative",
            "/home/../etc",
            "/home/",
            "/bad\nname",
            "/a//b",
        ] {
            let mut settings = WorkspacesSettings {
                container_directory: invalid.into(),
            };
            settings.sanitize();
            assert_eq!(settings.container_directory, "/home", "{invalid:?}");
        }
    }
}
