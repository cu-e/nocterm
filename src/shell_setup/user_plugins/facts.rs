//! What the host knows of an installed user plugin, for the inspector of the bar editor.
use gpui_kit::{App, Entity};
use nocterm_layout_ui::{UserFacts, UserPluginFacts};
use nocterm_plugin_host::PluginHost;
use nocterm_plugin_model::PluginId;

/// The facts of the plugins the host holds.
pub(super) struct HostFacts {
    host: Entity<PluginHost>,
}

impl HostFacts {
    pub(super) fn new(host: Entity<PluginHost>) -> Self {
        Self { host }
    }
}

impl UserPluginFacts for HostFacts {
    fn facts(&self, plugin: &PluginId, cx: &App) -> Option<UserFacts> {
        let host = self.host.read(cx);
        let summary = host
            .plugins()
            .into_iter()
            .find(|summary| summary.id == *plugin)?;
        let publisher = host.manifest(plugin).map_or_else(
            || super::manager::publisher_of(plugin),
            |manifest| manifest.publisher().to_owned(),
        );
        Some(UserFacts {
            publisher,
            fingerprint: super::manager::fingerprint(&summary.hash),
            capabilities: host
                .granted(plugin)
                .iter()
                .map(|id| super::manager::host_words(id))
                .collect(),
        })
    }
}
