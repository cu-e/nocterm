//! The providers of one window, gathered for the plugin host.
use std::{rc::Rc, sync::Arc};

use gpui_kit::{AnyWindowHandle, App, Entity};
use nocterm_core::clock::WallClock;
use nocterm_files::TransferCounts;
use nocterm_monitor_ui::MonitorUsage;
use nocterm_plugin_api::{
    HostError,
    capability::clock::{ClockNow, Now},
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::{Observe, PluginProviders};
use nocterm_ui::notice::PluginNotices;
use nocterm_workspace::{ShellCapabilities, Workspace};

use crate::shell_setup::user_plugin_panel::Badges;

/// Capabilities the host answers itself, so a window has no provider for them.
#[cfg(test)]
pub(super) const HOST_HANDLED: &[&str] = &["storage.set", "storage.remove"];

/// Capabilities this build declares unavailable in every window. None: a capability without a
/// provider or a line here fails the coverage test, so a new one cannot be forgotten.
#[cfg(test)]
pub(super) const UNAVAILABLE: &[&str] = &[];

/// The observation and effect providers a window offers: what the user's own windows already
/// show, and nothing that starts work of its own.
pub(crate) fn window_providers(
    workspace: &Entity<Workspace>,
    window: AnyWindowHandle,
    usage: &MonitorUsage,
    counts: TransferCounts,
    clock: Arc<dyn WallClock>,
    badges: &Badges,
    cx: &mut App,
) -> PluginProviders {
    let notices = Rc::new(PluginNotices::new(window));
    let shell = Rc::new(ShellCapabilities::new(workspace, window, cx));
    PluginProviders::new()
        .with_clock(Rc::new(ClockObservation(clock)))
        .with_workspace(shell.clone())
        .with_open(shell.clone())
        .with_badge(Rc::new(badges.clone()))
        .with_surface(shell)
        .with_usage(Rc::new(usage.clone()))
        .with_counts(Rc::new(counts))
        .with_own_notices(notices.clone())
        .with_post(notices)
}

/// `clock.now`: the wall clock the application gave the shell, to the minute. Reading it starts
/// nothing and touches no entity.
struct ClockObservation(Arc<dyn WallClock>);

impl Observe<Now> for ClockObservation {
    fn sample(&self, _: &PluginId, _: &App) -> Result<ClockNow, HostError> {
        let now = self.0.now();
        Ok(ClockNow {
            unix_minutes: now.unix_seconds.div_euclid(60),
            utc_offset_seconds: now.utc_offset_seconds,
        })
    }
}
