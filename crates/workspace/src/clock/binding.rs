//! The Clock's plugin settings as the schema fields of the `[clock]` section.

use std::rc::Rc;

use nocterm_plugin_model::{FieldError, SettingValue, SettingValues};
use nocterm_store::SettingsHandle;
use nocterm_ui::{PluginSettingsSource, SectionBinding};

use super::{
    plugin::{self, DATE_KEY, FORMAT_KEY, SECONDS_KEY},
    settings::{ClockFormat, ClockSettings},
};

fn read(settings: &ClockSettings) -> SettingValues {
    let mut values = SettingValues::default();
    values.insert(
        FORMAT_KEY,
        SettingValue::Choice(settings.format.key().into()),
    );
    values.insert(SECONDS_KEY, SettingValue::Bool(settings.seconds));
    values.insert(DATE_KEY, SettingValue::Bool(settings.date));
    values
}

fn write(settings: &mut ClockSettings, key: &str, value: SettingValue) -> Result<(), FieldError> {
    match (key, value) {
        (FORMAT_KEY, SettingValue::Choice(chosen)) => {
            settings.format =
                ClockFormat::from_key(&chosen).ok_or_else(|| FieldError::UnknownOption {
                    key: FORMAT_KEY.to_string(),
                    option: chosen,
                })?;
        }
        (SECONDS_KEY, SettingValue::Bool(on)) => settings.seconds = on,
        (DATE_KEY, SettingValue::Bool(on)) => settings.date = on,
        (key, _) => return Err(FieldError::UnknownKey(key.to_string())),
    }
    Ok(())
}

/// Where the Clock's plugin settings are stored: section `[clock]` of `settings`. Every window
/// follows a change at once.
pub fn plugin_settings(settings: SettingsHandle) -> Rc<dyn PluginSettingsSource> {
    Rc::new(SectionBinding::<ClockSettings>::new(
        settings,
        &plugin::PLUGIN.settings,
        read,
        write,
    ))
}
