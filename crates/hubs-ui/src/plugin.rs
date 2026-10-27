//! What the hubs feature contributes to the window shell: the sessions panel, a system plugin
//! of the catalog in the application.
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, PanelChrome, PanelTraits, Placement,
    PluginId, SettingsSchema, SystemContribution, SystemPlugin, WidthClass, Zone,
};

/// The terminal sessions of a hub in the sidebar. It stays in the tray until the user places
/// it, or opens it with the action that shows the sessions.
pub const SESSIONS: SystemContribution = SystemContribution {
    id: ContributionId::new("hubs.sessions"),
    kind: ContributionKind::Panel,
    title: "Sessions",
    icon: Icon::SquareTerminal,
    placement: Placement {
        zone: Zone::Left,
        order: 70,
        group: Some("navigation"),
        auto: AutoPlace::Tray,
    },
    panel: Some(PanelTraits {
        width: WidthClass::Navigation,
        chrome: PanelChrome::Titled,
    }),
    preview: None,
    pinned: false,
};

/// Hubs: the terminal sessions on the machines of a nocterm hub.
pub static PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("hubs"),
    name: "Hubs",
    description: "Terminal sessions on the machines of a nocterm hub",
    required: false,
    contributions: &[SESSIONS],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;
