//! The selected workspace environment's resources in the status bar.
//!
//! A compact summary — CPU and memory by default — follows the active tab's
//! environment; clicking it opens supported CPU, memory, temperature,
//! disks, uptime and graphs of network and disk throughput. Collection runs
//! on the host itself through [`nocterm_monitor`]: while the details are
//! closed only what the summary shows is read, at the slow interval.
//!
//! Owns: selected monitor-source lifecycle, generation-bound sampling, resource-specific
//! history, status/details views and monitor settings controls.
//! Installation retains actual settings and optional supplied local host execution; models share it.
//! Does not own: metric parsing, host execution or session establishment.
//! Ports used: immutable workspace MonitorSource, scoped session HostExec and supplied
//! SettingsHandle; provided: SettingsPageSpec.
//! Settings section: `[monitor]`. Globals: none.
//! Invariants: one script collects the current metric union; eight retained records stay
//! distinct by surface and resource; unavailable projects never use a local fallback.
//! Container CPU uses a labeled one-CPU basis; cgroup memory never uses host capacity.
mod details;
mod installed;
pub use installed::{Installed, install};
mod graph;
mod model;
mod page;
mod settings;
mod source;
mod status;

use gpui_kit::{AppContext as _, Context, Window};
use nocterm_ui::SettingsPageSpec;
use nocterm_workspace::Workspace;

pub use page::MonitorPage;
pub use settings::{
    MONITOR_DETAIL_INTERVAL_RANGE, MONITOR_HISTORY_RANGE, MONITOR_INTERVAL_RANGE, MonitorSettings,
};

gpui_kit::actions!(
    monitor,
    [
        /// Open the host monitor's settings.
        OpenMonitorSettings,
    ]
);

/// Adds the monitor to the footer of `workspace`.
pub fn register(
    installed: &Installed,
    workspace: &mut Workspace,
    _window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let handle = cx.entity();
    let session = workspace.monitor_source(cx);
    let monitor = cx.new(|cx| model::HostMonitor::new(&handle, session, installed, cx));
    let status = cx.new(|cx| status::MonitorStatus::new(monitor, cx));
    workspace.add_leading_status_view(status, cx);
}

/// Supplies the lazy Monitor page to the application Settings host.
pub fn settings_page(installed: &Installed) -> SettingsPageSpec {
    let settings = installed.settings().clone();
    SettingsPageSpec::new("monitor", "Monitor", move |window, cx| {
        cx.new(|cx| MonitorPage::new(settings.clone(), window, cx))
    })
    .with_icon(nocterm_ui::IconName::Activity)
}

#[cfg(test)]
mod tests;
