//! The snippet library as a system plugin: one sidebar panel.
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, PanelChrome, PanelTraits, Placement,
    PluginId, SettingsSchema, SystemContribution, SystemPlugin, WidthClass, Zone,
};

/// The snippet library in the sidebar.
pub const LIBRARY: SystemContribution = SystemContribution {
    id: ContributionId::new("snippets.library"),
    kind: ContributionKind::Panel,
    title: "Snippets",
    icon: Icon::ScrollText,
    placement: Placement {
        zone: Zone::Left,
        order: 30,
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

/// Snippets: saved commands to run in the active terminal.
pub static PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("snippets"),
    name: "Snippets",
    description: "Saved commands to run in the active terminal",
    required: false,
    contributions: &[LIBRARY],
    settings_section: None,
    settings: SettingsSchema::NONE,
};

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;
