//! The containers list as a system plugin: one sidebar panel.
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, PanelChrome, PanelTraits, Placement,
    PluginId, SystemContribution, SystemPlugin, WidthClass, Zone,
};

/// The containers of the active host in the sidebar.
pub const LIST: SystemContribution = SystemContribution {
    id: ContributionId::new("containers.list"),
    kind: ContributionKind::Panel,
    title: "Containers",
    icon: Icon::Container,
    placement: Placement {
        zone: Zone::Left,
        order: 40,
        group: Some("navigation"),
        auto: AutoPlace::Show,
    },
    panel: Some(PanelTraits {
        width: WidthClass::Navigation,
        chrome: PanelChrome::Titled,
    }),
    preview: None,
    pinned: false,
};

/// Containers: the containers and images of the active host.
pub static PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("containers"),
    name: "Containers",
    description: "Containers and images of the active host",
    required: false,
    contributions: &[LIST],
};

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;
