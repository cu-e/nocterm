//! The status bar metrics as the schema field `status_bar` of the `monitor` plugin.
//!
//! The field is stored where it always was, in `[monitor].status_bar`; this module only maps
//! between the schema's option keys and [`MonitorMetric`].

use std::rc::Rc;

use nocterm_monitor::MonitorMetric;
use nocterm_plugin_model::{FieldError, SettingValue, SettingValues};
use nocterm_ui::{PluginSettingsSource, SectionBinding};

use crate::{Installed, MonitorSettings, plugin};

/// The option key of `metric`, equal to its serde name.
fn key_of(metric: MonitorMetric) -> &'static str {
    match metric {
        MonitorMetric::Cpu => "cpu",
        MonitorMetric::Cores => "cores",
        MonitorMetric::Memory => "memory",
        MonitorMetric::Swap => "swap",
        MonitorMetric::Load => "load",
        MonitorMetric::Uptime => "uptime",
        MonitorMetric::Temperature => "temperature",
        MonitorMetric::Disks => "disks",
        MonitorMetric::Network => "network",
        MonitorMetric::DiskIo => "disk_io",
    }
}

fn metric_of(key: &str) -> Option<MonitorMetric> {
    MonitorMetric::ALL
        .into_iter()
        .find(|metric| key_of(*metric) == key)
}

fn read(settings: &MonitorSettings) -> SettingValues {
    let mut values = SettingValues::default();
    values.insert(
        plugin::STATUS_BAR_KEY,
        SettingValue::Choices(
            settings
                .status_bar
                .iter()
                .map(|metric| key_of(*metric).to_string())
                .collect(),
        ),
    );
    values
}

fn write(settings: &mut MonitorSettings, key: &str, value: SettingValue) -> Result<(), FieldError> {
    match (key, value) {
        (plugin::STATUS_BAR_KEY, SettingValue::Choices(chosen)) => {
            let metrics = chosen
                .iter()
                .map(|key| {
                    metric_of(key).ok_or_else(|| FieldError::UnknownOption {
                        key: plugin::STATUS_BAR_KEY.to_string(),
                        option: key.clone(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            settings.status_bar = metrics;
            Ok(())
        }
        (key, _) => Err(FieldError::UnknownKey(key.to_string())),
    }
}

/// Where the Monitor's plugin settings are stored: section `[monitor]` of the installed
/// settings. The binding runs no program; a running monitor follows a change as it does for the
/// Monitor page.
pub fn plugin_settings(installed: &Installed) -> Rc<dyn PluginSettingsSource> {
    Rc::new(SectionBinding::<MonitorSettings>::new(
        installed.settings().clone(),
        &plugin::PLUGIN.settings,
        read,
        write,
    ))
}
