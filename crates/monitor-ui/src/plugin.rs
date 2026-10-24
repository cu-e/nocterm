//! The host monitor as a system plugin: one status bar widget.
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, Placement, PluginId, SettingsSchema,
    SystemContribution, SystemPlugin, Zone,
};

/// The resource summary of the active environment in the status bar.
pub const SUMMARY: SystemContribution = SystemContribution {
    id: ContributionId::new("monitor.summary"),
    kind: ContributionKind::Widget,
    title: "Monitor",
    icon: Icon::Activity,
    placement: Placement {
        zone: Zone::Left,
        order: 60,
        group: None,
        auto: AutoPlace::Show,
    },
    panel: None,
    preview: None,
    pinned: false,
};

/// Monitor: CPU and memory of the active environment.
pub static PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("monitor"),
    name: "Monitor",
    description: "CPU and memory of the active environment in the status bar",
    required: false,
    contributions: &[SUMMARY],
    settings_section: None,
    settings: SettingsSchema::NONE,
};
