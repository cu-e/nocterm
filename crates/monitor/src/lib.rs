//! Host resource monitoring, independent of any UI.
//!
//! Owns: host and own-cgroup collection requests, bounded framed parsing, controller
//! membership/mount validation, rates and bounded history.
//! Does not own: native process execution, GUI timers, host connections or views.
//! Ports used: HostExec. Ports provided: none. Settings section: none. Globals: none.
//! Invariants: only requested metrics enter the collection script; history is
//! capacity bounded and successive counter samples determine rates. Container CPU is
//! measured against one CPU; memory limits may be unlimited or unobservable. Host-only
//! metrics never substitute for missing cgroup counters.
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

//! Settings schema: owned here, including defaults and sanitization; controls stay in the feature.
//! Owns: host metric scripts, parsing, bounded histories and monitor settings rules.
//! Does not own: GPUI views, native host execution or settings publication.
//! Ports: HostExec. Settings: monitor. Globals: none.
//! Invariant: sampling and schema rules accept supplied facts without native I/O.
pub mod format;

mod container;
mod frame;
pub use container::{ContainerCounters, ContainerSnapshot};
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

mod settings;
pub use settings::*;
