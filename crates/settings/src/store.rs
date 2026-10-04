//! `settings.toml` on disk.

use std::path::{Path, PathBuf};

use nocterm_core::{PersistError, persist};

use crate::Settings;

/// The user's settings file.
///
/// The file holds only what differs from the defaults, so a default that
/// improves in a later release reaches everyone who never touched it.
#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the settings. A file that does not exist yields the defaults.
    pub fn load(&self) -> Result<Settings, PersistError> {
        Ok(persist::load::<Settings>(&self.path)?
            .unwrap_or_default()
            .sanitized())
    }

    /// Writes the settings, keeping the comments already in the file.
    pub fn save(&self, settings: &Settings) -> Result<(), PersistError> {
        let mut table = to_table(settings);
        prune_defaults(&mut table, &to_table(&Settings::default()));
        persist::save_preserving(&self.path, &table)
    }
}

fn to_table(settings: &Settings) -> toml::Table {
    toml::Table::try_from(settings).expect("settings serialise to a TOML table")
}

/// Removes every entry of `table` that equals its counterpart in `defaults`.
fn prune_defaults(table: &mut toml::Table, defaults: &toml::Table) {
    table.retain(|key, value| match (value, defaults.get(key)) {
        (toml::Value::Table(table), Some(toml::Value::Table(defaults))) => {
            prune_defaults(table, defaults);
            !table.is_empty()
        }
        (value, Some(default)) => value != default,
        (_, None) => true,
    });
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::CursorShape;

    #[test]
    fn theme_names_round_trip_prune_and_sanitize() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        assert_eq!(Settings::default().appearance.light_theme, None);
        assert_eq!(Settings::default().appearance.dark_theme, None);
        file.save(&Settings::default()).unwrap();
        assert_eq!(fs::read_to_string(file.path()).unwrap(), "");
        let mut settings = Settings::default();
        settings.appearance.light_theme = Some("My Light".into());
        settings.appearance.dark_theme = Some("My Dark".into());
        file.save(&settings).unwrap();
        assert_eq!(file.load().unwrap(), settings);
        settings.appearance.light_theme = Some("  ".into());
        settings.appearance.dark_theme = Some(" My Dark ".into());
        let settings = settings.sanitized();
        assert_eq!(settings.appearance.light_theme, None);
        assert_eq!(settings.appearance.dark_theme.as_deref(), Some("My Dark"));
        assert!(toml::from_str::<Settings>("[appearance]\nunknown_theme = 'X'\n").is_err());
        file.save(&Settings::default()).unwrap();
        assert_eq!(fs::read_to_string(file.path()).unwrap(), "");
    }

    #[test]
    fn missing_file_loads_the_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));

        assert_eq!(file.load().unwrap(), Settings::default());
    }

    #[test]
    fn only_changed_settings_are_written() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        let mut settings = Settings::default();
        settings.terminal.cursor_shape = CursorShape::Underline;
        settings.terminal.font_size = Some(15.0);

        file.save(&settings).unwrap();

        assert_eq!(
            fs::read_to_string(file.path()).unwrap(),
            "[terminal]\ncursor_shape = \"underline\"\nfont_size = 15.0\n"
        );
        assert_eq!(file.load().unwrap(), settings);
    }

    #[test]
    fn restoring_a_default_removes_it_from_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        let mut settings = Settings::default();
        settings.terminal.copy_on_select = true;
        file.save(&settings).unwrap();

        file.save(&Settings::default()).unwrap();

        assert_eq!(fs::read_to_string(file.path()).unwrap(), "");
    }

    #[test]
    fn saving_keeps_the_users_comments() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        fs::write(
            file.path(),
            "[terminal]\n# Easier on the eyes.\nfont_size = 15.0\n",
        )
        .unwrap();
        let mut settings = file.load().unwrap();
        settings.terminal.font_size = Some(16.0);

        file.save(&settings).unwrap();

        assert_eq!(
            fs::read_to_string(file.path()).unwrap(),
            "[terminal]\n# Easier on the eyes.\nfont_size = 16.0\n"
        );
    }

    #[test]
    fn only_changed_ai_settings_are_written() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        let mut settings = Settings::default();
        settings.ai.approval.terminal_write = crate::ApprovalPolicy::Allow;
        settings.ai.agents.insert(
            "hermes".to_owned(),
            crate::AgentServerSettings {
                enabled: false,
                ..Default::default()
            },
        );
        settings.ai.agents.insert(
            "mine".to_owned(),
            crate::AgentServerSettings {
                command: Some("/opt/mine".to_owned()),
                ..Default::default()
            },
        );

        file.save(&settings).unwrap();

        assert_eq!(
            fs::read_to_string(file.path()).unwrap(),
            "[ai.agents.hermes]\nenabled = false\n\n[ai.agents.mine]\ncommand = \"/opt/mine\"\n\n[ai.approval]\nterminal_write = \"allow\"\n"
        );
        assert_eq!(file.load().unwrap(), settings);
    }

    #[test]
    fn loading_sanitizes_out_of_range_values() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        fs::write(file.path(), "[terminal]\nfont_size = 1.0\n").unwrap();

        assert_eq!(file.load().unwrap().terminal.font_size, Some(6.0));
    }
}
