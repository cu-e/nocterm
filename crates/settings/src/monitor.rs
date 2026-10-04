//! `[monitor]`: the host resource monitor in the status bar.

use std::ops::RangeInclusive;

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

impl MonitorSettings {
    pub(crate) fn sanitize(&mut self) {
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

/// One thing the monitor can report about a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MonitorMetric {
    /// Total processor load.
    Cpu,
    /// Load of every processor core.
    Cores,
    /// Memory in use.
    Memory,
    /// Swap or page file in use.
    Swap,
    /// Load average (Linux).
    Load,
    /// Time since the host started.
    Uptime,
    /// Temperature sensors.
    Temperature,
    /// Space used on each file system.
    Disks,
    /// Network throughput.
    Network,
    /// Disk read and write throughput.
    DiskIo,
}

impl MonitorMetric {
    pub const ALL: [Self; 10] = [
        Self::Cpu,
        Self::Cores,
        Self::Memory,
        Self::Swap,
        Self::Load,
        Self::Uptime,
        Self::Temperature,
        Self::Disks,
        Self::Network,
        Self::DiskIo,
    ];

    /// The name a person reads.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Cores => "CPU cores",
            Self::Memory => "Memory",
            Self::Swap => "Swap",
            Self::Load => "Load average",
            Self::Uptime => "Uptime",
            Self::Temperature => "Temperature",
            Self::Disks => "Disk space",
            Self::Network => "Network",
            Self::DiskIo => "Disk I/O",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Settings;

    #[test]
    fn defaults_show_cpu_and_memory_and_every_detail() {
        let settings: Settings = toml::from_str("").unwrap();
        let monitor = settings.monitor;
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
        let settings: Settings =
            toml::from_str("[monitor]\nstatus_bar = [\"network\", \"disk_io\", \"cpu\"]\n")
                .unwrap();
        assert_eq!(
            settings.monitor.status_bar,
            [
                MonitorMetric::Network,
                MonitorMetric::DiskIo,
                MonitorMetric::Cpu
            ]
        );
    }

    #[test]
    fn sanitizing_clamps_intervals_and_drops_repeats() {
        let mut settings = Settings::default();
        settings.monitor.interval_secs = 0;
        settings.monitor.detail_interval_secs = 1000;
        settings.monitor.history_points = 1;
        settings.monitor.status_bar =
            vec![MonitorMetric::Cpu, MonitorMetric::Swap, MonitorMetric::Cpu];

        let monitor = settings.sanitized().monitor;

        assert_eq!(monitor.interval_secs, 1);
        assert_eq!(monitor.detail_interval_secs, 60);
        assert_eq!(monitor.history_points, 10);
        assert_eq!(
            monitor.status_bar,
            [MonitorMetric::Cpu, MonitorMetric::Swap]
        );
    }
}
