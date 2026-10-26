//! Workspaces as a system plugin: one sidebar panel.
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, PanelChrome, PanelTraits, Placement,
    PluginId, SettingsSchema, SystemContribution, SystemPlugin, WidthClass, Zone,
};

/// The saved workspaces in the sidebar.
pub const WORKSPACES: SystemContribution = SystemContribution {
    id: ContributionId::new("projects.workspaces"),
    kind: ContributionKind::Panel,
    title: "Workspaces",
    icon: Icon::Layers,
    placement: Placement {
        zone: Zone::Left,
        order: 50,
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

/// Workspaces: saved workspaces on a server or this computer.
pub static PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("projects"),
    name: "Workspaces",
    description: "Saved workspaces on a server or this computer",
    required: false,
    contributions: &[WORKSPACES],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: Some("workspaces"),
};
