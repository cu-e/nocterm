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
    ///
    /// Every top-level table is read on its own: a mistake in one section
    /// replaces only that section with its defaults and is reported in
    /// [`LoadedSettings::errors`]. Only a file that is not TOML at all, or
    /// cannot be read, is an error.
    pub fn load(&self) -> Result<LoadedSettings, PersistError> {
        let table = persist::load::<toml::Table>(&self.path)?.unwrap_or_default();
        let mut valid = toml::Table::new();
        let mut errors = Vec::new();
        for (key, value) in table {
            match read_section(&key, &value) {
                Ok(()) => {
                    valid.insert(key, value);
                }
                Err(error) => errors.push(SectionError { key, error }),
            }
        }
        let settings = toml::Value::Table(valid)
            .try_into::<Settings>()
            .expect("sections that parse alone parse together");
        Ok(LoadedSettings {
            settings: settings.sanitized(),
            errors,
        })
    }

    /// Writes the settings, keeping the comments already in the file.
    ///
    /// A section the file holds but that could not be read is left as it is,
    /// unless `settings` changed it from its defaults.
    pub fn save(&self, settings: &Settings) -> Result<(), PersistError> {
        let mut table = to_table(settings);
        prune_defaults(&mut table, &to_table(&Settings::default()));
        if let Ok(Some(current)) = persist::load::<toml::Table>(&self.path) {
            for (key, value) in current {
                if !table.contains_key(&key) && read_section(&key, &value).is_err() {
                    table.insert(key, value);
                }
            }
        }
        persist::save_preserving(&self.path, &table)
    }
}

/// Settings read from a file, with the sections that had to be replaced by
/// their defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedSettings {
    pub settings: Settings,
    pub errors: Vec<SectionError>,
}

/// A top-level table of `settings.toml` that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionError {
    /// The table's name, such as `monitor`.
    pub key: String,
    pub error: String,
}

impl std::fmt::Display for SectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]: {}", self.key, self.error)
    }
}

impl SectionError {
    /// Whether the error still describes the file after `before` was
    /// replaced by `after`: a section that was changed has been rewritten.
    pub fn outlives(&self, before: &Settings, after: &Settings) -> bool {
        to_table(before).get(&self.key) == to_table(after).get(&self.key)
    }
}

/// Checks that one top-level table reads as its section.
fn read_section(key: &str, value: &toml::Value) -> Result<(), String> {
    let single = toml::Table::from_iter([(key.to_owned(), value.clone())]);
    toml::Value::Table(single)
        .try_into::<Settings>()
        .map(drop)
        .map_err(|error| error.to_string().trim().to_owned())
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
        assert_eq!(file.load().unwrap().settings, settings);
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

        assert_eq!(file.load().unwrap().settings, Settings::default());
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
        assert_eq!(file.load().unwrap().settings, settings);
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
        let mut settings = file.load().unwrap().settings;
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
        assert_eq!(file.load().unwrap().settings, settings);
    }

    #[test]
    fn loading_sanitizes_out_of_range_values() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        fs::write(file.path(), "[terminal]\nfont_size = 1.0\n").unwrap();

        assert_eq!(file.load().unwrap().settings.terminal.font_size, Some(6.0));
    }

    #[test]
    fn a_broken_section_falls_back_alone_and_survives_saving() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        let broken = "[monitor]\n# Mine.\ninterval_secs = 5\nintervall = 3\n";
        fs::write(
            file.path(),
            format!("[terminal]\nfont_size = 15.0\n\n{broken}\n[future]\nflag = true\n"),
        )
        .unwrap();

        let loaded = file.load().unwrap();
        assert_eq!(loaded.settings.terminal.font_size, Some(15.0));
        assert_eq!(loaded.settings.monitor, crate::MonitorSettings::default());
        let keys: Vec<_> = loaded
            .errors
            .iter()
            .map(|error| error.key.as_str())
            .collect();
        assert_eq!(keys, ["future", "monitor"]);
        assert!(
            loaded.errors[1].error.contains("intervall"),
            "{}",
            loaded.errors[1]
        );

        let mut settings = loaded.settings;
        settings.terminal.font_size = Some(16.0);
        file.save(&settings).unwrap();
        let text = fs::read_to_string(file.path()).unwrap();
        assert!(text.contains("font_size = 16.0"), "{text}");
        assert!(text.contains(broken), "{text}");
        assert!(text.contains("[future]\nflag = true"), "{text}");
        assert_eq!(file.load().unwrap().errors.len(), 2);
    }

    #[test]
    fn changing_a_broken_section_rewrites_it() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        fs::write(file.path(), "[terminal]\nfont_szie = 15.0\n").unwrap();
        let loaded = file.load().unwrap();
        let mut settings = loaded.settings.clone();
        settings.terminal.copy_on_select = true;
        assert!(!loaded.errors[0].outlives(&loaded.settings, &settings));
        assert!(loaded.errors[0].outlives(&loaded.settings, &loaded.settings));

        file.save(&settings).unwrap();

        assert_eq!(
            fs::read_to_string(file.path()).unwrap(),
            "[terminal]\ncopy_on_select = true\n"
        );
        assert!(file.load().unwrap().errors.is_empty());
    }

    #[test]
    fn a_file_that_is_not_toml_is_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        fs::write(file.path(), "[terminal\n").unwrap();
        assert!(file.load().is_err());
    }
}
