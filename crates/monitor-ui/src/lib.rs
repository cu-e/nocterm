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
mod status;

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Context, Window};
use nocterm_session::HostExec;
use nocterm_workspace::{SettingsPageSpec, Workspace};

pub use page::MonitorPage;

gpui_kit::actions!(
    monitor,
    [
        /// Open the host monitor's settings.
        OpenMonitorSettings,
    ]
);

/// Installs the program runner for this computer, which the monitor uses
/// whenever no remote session is active.
pub fn init(local: Arc<dyn HostExec>, cx: &mut App) {
    cx.set_global(model::LocalExec(local));
}

/// Adds the monitor to the footer of `workspace`.
pub fn register(workspace: &mut Workspace, _window: &mut Window, cx: &mut Context<Workspace>) {
    let handle = cx.entity();
    let session = workspace.active_session(cx);
    let local = model::local_exec(cx);
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
