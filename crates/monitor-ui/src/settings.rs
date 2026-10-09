//! `[monitor]`: the host resource monitor in the status bar.

use std::ops::RangeInclusive;

use nocterm_monitor::MonitorMetric;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Status bar refresh intervals a user may pick, in seconds.
pub const MONITOR_INTERVAL_RANGE: RangeInclusive<u32> = 1..=3600;
/// Details refresh intervals a user may pick, in seconds.
pub const MONITOR_DETAIL_INTERVAL_RANGE: RangeInclusive<u32> = 1..=60;
/// Graph lengths a user may pick, in samples.
pub const MONITOR_HISTORY_RANGE: RangeInclusive<u32> = 10..=600;

/// The resources of the active host, shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct MonitorSettings {
    /// Show the active host's resources in the status bar.
    pub enabled: bool,
    /// Watch this computer while no remote session is active.
    pub local: bool,
    /// Seconds between updates while the details are closed. Only what the
    /// status bar shows is collected then.
    #[schemars(extend("minimum" = MONITOR_INTERVAL_RANGE.start(), "maximum" = MONITOR_INTERVAL_RANGE.end()))]
    pub interval_secs: u32,
    /// Seconds between updates while the details are open.
    #[schemars(extend("minimum" = MONITOR_DETAIL_INTERVAL_RANGE.start(), "maximum" = MONITOR_DETAIL_INTERVAL_RANGE.end()))]
    pub detail_interval_secs: u32,
    /// Samples the graphs in the details keep.
    #[schemars(extend("minimum" = MONITOR_HISTORY_RANGE.start(), "maximum" = MONITOR_HISTORY_RANGE.end()))]
    pub history_points: u32,
    /// What the status bar shows, in this order.
    pub status_bar: Vec<MonitorMetric>,
    /// What the details show.
    pub details: Vec<MonitorMetric>,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            local: true,
            interval_secs: 30,
            detail_interval_secs: 2,
            history_points: 90,
            status_bar: vec![MonitorMetric::Cpu, MonitorMetric::Memory],
            details: MonitorMetric::ALL.to_vec(),
        }
    }
}

impl nocterm_settings::SettingsSection for MonitorSettings {
    const KEY: &'static str = "monitor";

    fn sanitize(&mut self) {
        let clamp =
            |value: u32, range: RangeInclusive<u32>| value.clamp(*range.start(), *range.end());
        self.interval_secs = clamp(self.interval_secs, MONITOR_INTERVAL_RANGE);
        self.detail_interval_secs = clamp(self.detail_interval_secs, MONITOR_DETAIL_INTERVAL_RANGE);
        self.history_points = clamp(self.history_points, MONITOR_HISTORY_RANGE);
        dedup(&mut self.status_bar);
        dedup(&mut self.details);
    }
}

/// Keeps the first occurrence of every metric, in order.
fn dedup(metrics: &mut Vec<MonitorMetric>) {
    let mut seen = Vec::with_capacity(metrics.len());
    metrics.retain(|metric| {
        let first = !seen.contains(metric);
        seen.push(*metric);
        first
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_settings::SettingsSection as _;

    #[test]
    fn defaults_show_cpu_and_memory_and_every_detail() {
        let monitor: MonitorSettings = toml::from_str("").unwrap();
        assert_eq!(
            monitor.status_bar,
            [MonitorMetric::Cpu, MonitorMetric::Memory]
        );
        assert_eq!(monitor.details, MonitorMetric::ALL);
        assert_eq!(
            (monitor.interval_secs, monitor.detail_interval_secs),
            (30, 2)
        );
    }

    #[test]
    fn metrics_are_read_by_name_in_order() {
        let monitor: MonitorSettings =
            toml::from_str("status_bar = [\"network\", \"disk_io\", \"cpu\"]\n").unwrap();
        assert_eq!(
            monitor.status_bar,
            [
                MonitorMetric::Network,
                MonitorMetric::DiskIo,
                MonitorMetric::Cpu
            ]
        );
    }

    #[test]
    fn sanitizing_clamps_intervals_and_drops_repeats() {
        let mut monitor = MonitorSettings {
            interval_secs: 0,
            detail_interval_secs: 1000,
            history_points: 1,
            status_bar: vec![MonitorMetric::Cpu, MonitorMetric::Swap, MonitorMetric::Cpu],
            ..MonitorSettings::default()
        };

        monitor.sanitize();

        assert_eq!(monitor.interval_secs, 1);
        assert_eq!(monitor.detail_interval_secs, 60);
        assert_eq!(monitor.history_points, 10);
        assert_eq!(
            monitor.status_bar,
            [MonitorMetric::Cpu, MonitorMetric::Swap]
        );
    }
}
