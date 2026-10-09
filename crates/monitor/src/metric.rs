//! What the monitor can report about a host.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
