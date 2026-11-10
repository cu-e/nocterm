//! The monitor's latest reading, offered to user plugins that were granted `monitor.usage`.
//!
//! Passive: the provider only looks at the monitor the status bar widget already created. A
//! window whose layout does not place the widget, a disabled plugin and a monitor that is off
//! have no reading to give, so the answer is `unavailable`; the provider never starts a watch.
use std::{cell::RefCell, rc::Rc};

use gpui_kit::{App, Context, Entity, WeakEntity, Window};
use nocterm_plugin_api::{
    HostError,
    capability::monitor::{Usage, UsageNow},
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::Observe;
use nocterm_workspace::{RegistrationError, Workspace};

use crate::{
    Installed,
    model::{HostMonitor, Status},
};

/// The CPU and memory use of the window's monitored host.
#[derive(Clone, Default)]
pub struct MonitorUsage {
    monitor: Rc<RefCell<Option<WeakEntity<HostMonitor>>>>,
}

impl MonitorUsage {
    pub fn new() -> Self {
        Self::default()
    }

    fn publish(&self, monitor: &Entity<HostMonitor>) {
        *self.monitor.borrow_mut() = Some(monitor.downgrade());
    }
}

/// [`register`](crate::register), also letting `usage` read the monitor the widget creates.
pub fn register_observed(
    installed: &Installed,
    usage: &MonitorUsage,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), RegistrationError> {
    let usage = usage.clone();
    crate::register_with(
        installed,
        move |monitor| usage.publish(monitor),
        workspace,
        window,
        cx,
    )
}

impl Observe<UsageNow> for MonitorUsage {
    fn sample(&self, _: &PluginId, cx: &App) -> Result<Usage, HostError> {
        let monitor = self
            .monitor
            .borrow()
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .ok_or(HostError::Unavailable)?;
        let monitor = monitor.read(cx);
        if *monitor.status() != Status::Running {
            return Err(HostError::Unavailable);
        }
        let snapshot = monitor.sampler().ok_or(HostError::Unavailable)?.snapshot();
        let cpu = snapshot.cpu.ok_or(HostError::Unavailable)?;
        let memory = snapshot.memory.ok_or(HostError::Unavailable)?.percent();
        if !cpu.is_finite() || !memory.is_finite() {
            return Err(HostError::Unavailable);
        }
        Ok(Usage {
            cpu_percent: cpu.clamp(0.0, 100.0),
            memory_percent: memory.clamp(0.0, 100.0),
        })
    }
}
