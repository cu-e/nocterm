//! Host resource monitoring, independent of any UI.
//!
//! Owns: collection requests, framed counter parsing, rates and bounded history.
//! Does not own: native process execution, GUI timers, host connections or views.
//! Ports used: HostExec. Ports provided: none. Settings section: none. Globals: none.
//! Invariants: only requested metrics enter the collection script; history is
//! capacity bounded and successive counter samples determine rates.
//!
//! A host is asked about itself through a [`HostExec`]: one long-running
//! script per watch, written for the host's [`Platform`], prints a frame of
//! raw counters every interval. This crate builds those scripts, parses their
//! frames into [`Reading`]s and turns successive readings into a [`Snapshot`]
//! of loads and rates, with a bounded [`History`] for graphs.
//!
//! Only the metrics asked for are collected: a script for the status bar
//! alone reads two files per frame and forks once, for `sleep`.
//!
//! [`HostExec`]: nocterm_session::HostExec

pub mod format;
mod frame;
mod history;
mod linux;
mod metric;
mod metrics;
mod platform;
mod reading;
mod sampler;
mod watch;
mod windows;

pub use history::{History, Series};
pub use metrics::MetricSet;
pub use platform::{Platform, detect, local_platform};
pub use reading::{
    CpuTimes, DiskCounters, Filesystem, HostInfo, NetworkCounters, Reading, Temperature,
};
pub use sampler::{DiskRates, NetworkRates, Sampler, Snapshot, Usage};
pub use watch::Watch;

pub use metric::MonitorMetric;
