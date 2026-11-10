//! What the host is built from and called with: its ports, a prepared plugin and a call.

use std::{rc::Rc, sync::Arc};

use gpui::WindowId;
use nocterm_plugin_api::{
    ContributionContext, LocalId, Package, PluginInstance, PluginPackages, PluginRuntime,
};
use nocterm_plugin_model::PluginId;

use crate::{PluginData, PluginRecords};

/// What the host is built from. The application composes these; the host never looks them up.
pub struct HostPorts {
    pub records: Rc<dyn PluginRecords>,
    pub data: Rc<dyn PluginData>,
    pub packages: Arc<dyn PluginPackages>,
    pub runtime: Arc<dyn PluginRuntime>,
    /// Sends `net.request` effects; without it they are `unavailable`.
    pub net: Option<Arc<dyn nocterm_plugin_api::PluginNet>>,
}

/// Immutable package bytes and the instance constructed from them on a background executor.
pub struct PreparedPlugin {
    package: Package,
    instance: Option<Box<dyn PluginInstance>>,
}
impl PreparedPlugin {
    pub fn new(package: Package, instance: Option<Box<dyn PluginInstance>>) -> Self {
        Self { package, instance }
    }
    pub fn package(&self) -> &Package {
        &self.package
    }
    pub(crate) fn into_parts(self) -> (Package, Option<Box<dyn PluginInstance>>) {
        (self.package, self.instance)
    }
}

/// One contribution of one plugin in one window, asking for a view or reporting an event.
#[derive(Clone, Debug)]
pub struct Call {
    pub window: WindowId,
    pub plugin: PluginId,
    pub local: LocalId,
    pub context: ContributionContext,
    /// Whether the contribution's panel has focus, which samples at the faster cadence.
    pub in_focus: bool,
    /// The plugin's terminal whose bar the call draws or reports (`terminals.bar`), which the
    /// snapshot names; `None` for the contribution's own views.
    pub terminal: Option<String>,
}
