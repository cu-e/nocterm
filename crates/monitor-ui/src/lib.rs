//! The active host's resources at the start of the status bar.
//!
//! A compact summary — CPU and memory by default — follows the active tab's
//! host; clicking it opens details with every core, memory, temperatures,
//! disks, uptime and graphs of network and disk throughput. Collection runs
//! on the host itself through [`nocterm_monitor`]: while the details are
//! closed only what the summary shows is read, at the slow interval.
mod details;
mod graph;
mod model;
mod page;
mod settings;
mod status;

use gpui_kit::{AppContext as _, Context, Window};
use nocterm_workspace::{SettingsPageSpec, Workspace};

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
pub fn register(workspace: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) {
    nocterm_ui::register_setting::<crate::MonitorSettings>(cx);
    let handle = cx.entity();
    let session = workspace.active_session(cx);
    let local = nocterm_workspace::host::local_exec(cx);
    let monitor = cx.new(|cx| model::HostMonitor::new(&handle, session, local, cx));
    let status = cx.new(|cx| status::MonitorStatus::new(monitor, cx));
    workspace.add_leading_status_view(status, cx);
}

/// Supplies the lazy Monitor page to the application Settings host.
pub fn settings_page() -> SettingsPageSpec {
    SettingsPageSpec::new("monitor", "Monitor", |window, cx| {
        cx.new(|cx| MonitorPage::new(window, cx))
    })
    .with_icon(nocterm_ui::IconName::Activity)
}

#[cfg(test)]
mod tests;
