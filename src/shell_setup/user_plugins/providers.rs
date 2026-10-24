//! The providers of one window, gathered for the plugin host.
use std::rc::Rc;

use gpui_kit::{AnyWindowHandle, App, Entity};
use nocterm_files::TransferCounts;
use nocterm_monitor_ui::MonitorUsage;
use nocterm_plugin_ports::PluginProviders;
use nocterm_ui::notice::PluginNotices;
use nocterm_workspace::{ShellCapabilities, Workspace};

/// The observation and effect providers a window offers: what the user's own windows already
/// show, and nothing that starts work of its own.
pub(crate) fn window_providers(
    workspace: &Entity<Workspace>,
    window: AnyWindowHandle,
    usage: &MonitorUsage,
    counts: TransferCounts,
    cx: &mut App,
) -> PluginProviders {
    let notices = Rc::new(PluginNotices::new(window));
    PluginProviders::new()
        .with_workspace(Rc::new(ShellCapabilities::new(workspace, cx)))
        .with_usage(Rc::new(usage.clone()))
        .with_counts(Rc::new(counts))
        .with_own_notices(notices.clone())
        .with_post(notices)
}
