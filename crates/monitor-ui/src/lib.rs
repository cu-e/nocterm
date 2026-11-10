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
//! SettingsHandle; provided: SettingsPageSpec, the `monitor` system plugin
//! (`plugin::PLUGIN`) with its status bar widget and its "Status bar" settings schema, and
//! `plugin_settings` (a `SectionBinding` over `[monitor]`) and, behind `user-plugins`, the
//! `MonitorUsage` provider of `monitor.usage` for user plugins (it only reads the monitor the
//! widget created, so a disabled plugin or an unplaced widget answers `unavailable`).
//! Settings section: `[monitor]`. Globals: none.
//! Invariants: one script collects the current metric union; eight retained records stay
//! distinct by surface and resource; unavailable projects never use a local fallback.
//! Container CPU uses a labeled one-CPU basis; cgroup memory never uses host capacity.
mod details;
mod installed;
pub use installed::{Installed, install};
mod graph;
#[cfg(feature = "user-plugins")]
mod host_capabilities;
mod model;
mod page;
pub mod plugin;
mod settings;
mod source;
mod status;
mod status_bar_settings;

use gpui_kit::{AppContext as _, Context, Entity, Window};
use nocterm_ui::SettingsPageSpec;
use nocterm_workspace::{RegistrationError, WidgetRegistration, Workspace};

#[cfg(feature = "user-plugins")]
pub use host_capabilities::{MonitorUsage, register_observed};
pub use page::MonitorPage;
pub use settings::{
    MONITOR_DETAIL_INTERVAL_RANGE, MONITOR_HISTORY_RANGE, MONITOR_INTERVAL_RANGE, MonitorSettings,
};
pub use status_bar_settings::plugin_settings;

gpui_kit::actions!(
    monitor,
    [
        /// Open the host monitor's settings.
        OpenMonitorSettings,
    ]
);

/// Adds the monitor to the footer of `workspace`.
///
/// The widget creates the monitor, which starts reading the host: a window without the
/// registration runs no program.
pub fn register(
    installed: &Installed,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), RegistrationError> {
    register_with(installed, |_| {}, workspace, window, cx)
}

/// [`register`], handing each monitor the widget creates to `created`.
fn register_with(
    installed: &Installed,
    created: impl Fn(&Entity<model::HostMonitor>) + 'static,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), RegistrationError> {
    let installed = installed.clone();
    workspace.register_widget(
        WidgetRegistration::zone_aware(&plugin::SUMMARY, move |workspace, _, cx| {
            let session = workspace.monitor_source(cx);
            let handle = cx.entity();
            let monitor = cx.new(|cx| model::HostMonitor::new(&handle, session, &installed, cx));
            created(&monitor);
            cx.new(|cx| status::MonitorStatus::new(monitor, cx))
        }),
        window,
        cx,
    )
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
