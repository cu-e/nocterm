use gpui_kit::{App, BorrowAppContext as _, Global};
use nocterm_settings::{Settings, SettingsFile};

/// The user's settings, as an application global, and the file they live in.
pub struct SettingsStore {
    settings: Settings,
    file: Option<SettingsFile>,
}

impl SettingsStore {
    /// Settings backed by `file`: every change is written to it.
    pub fn new(settings: Settings, file: SettingsFile) -> Self {
        Self {
            settings: settings.sanitized(),
            file: Some(file),
        }
    }

    /// Settings that live only as long as the process.
    pub fn in_memory(settings: Settings) -> Self {
        Self {
            settings: settings.sanitized(),
            file: None,
        }
    }

    /// Whether changes can be saved across launches.
    pub fn is_persistent(&self) -> bool {
        self.file.is_some()
    }

    fn save(&mut self, settings: Settings) -> Result<(), String> {
        if let Some(file) = &self.file {
            file.save(&settings).map_err(|error| error.to_string())?;
        }
        self.settings = settings;
        Ok(())
    }
}

impl Global for SettingsStore {}

/// Read access to the settings from any context.
pub trait ActiveSettings {
    fn settings(&self) -> &Settings;
}

impl ActiveSettings for App {
    fn settings(&self) -> &Settings {
        &self.global::<SettingsStore>().settings
    }
}

/// Changes the settings and saves them.
///
/// Observers of [`SettingsStore`] run only when something actually changed.
pub fn update_settings(cx: &mut App, edit: impl FnOnce(&mut Settings)) -> Result<(), String> {
    let mut settings = cx.settings().clone();
    edit(&mut settings);
    let settings = settings.sanitized();
    if &settings == cx.settings() {
        return Ok(());
    }

    // Persist before publishing, so a failed write leaves the active values intact.
    cx.update_global::<SettingsStore, _>(|store, _| store.save(settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_save_leaves_active_settings_intact() {
        let directory = tempfile::tempdir().unwrap();
        let mut store =
            SettingsStore::new(Settings::default(), SettingsFile::new(directory.path()));
        let mut changed = Settings::default();
        changed.terminal.copy_on_select = true;
        assert!(store.save(changed).is_err());
        assert_eq!(store.settings, Settings::default());
    }
    #[test]
    fn saved_settings_survive_reload() {
        let directory = tempfile::tempdir().unwrap();
        let file = SettingsFile::new(directory.path().join("settings.toml"));
        let mut store = SettingsStore::new(Settings::default(), file.clone());
        let mut changed = Settings::default();
        changed.terminal.font_size = Some(18.);
        store.save(changed.clone()).unwrap();
        assert_eq!(file.load().unwrap(), changed);
        assert_eq!(store.settings, changed);
    }
}
