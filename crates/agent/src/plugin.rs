//! The agent chats as a required system plugin: one expandable side panel.
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, PanelChrome, PanelTraits, Placement,
    PluginId, SettingsSchema, SystemContribution, SystemPlugin, WidthClass, Zone,
};

/// The agent chats in the side panel, which draws its own header.
pub const CHATS: SystemContribution = SystemContribution {
    id: ContributionId::new("agent.chats"),
    kind: ContributionKind::Panel,
    title: "AI Agents",
    icon: Icon::PanelRight,
    placement: Placement {
        zone: Zone::Right,
        order: 40,
        group: Some("assistant"),
        auto: AutoPlace::Show,
    },
    panel: Some(PanelTraits {
        width: WidthClass::Assistant,
        chrome: PanelChrome::OwnHeader,
    }),
    preview: None,
    pinned: false,
};

/// AI Agents: chats with coding agents. It cannot be disabled; the master switch in the AI
/// settings withdraws its panel instead.
pub static PLUGIN: SystemPlugin = SystemPlugin {
    id: PluginId::new("agent"),
    name: "AI Agents",
    description: "Chats with coding agents in the side panel",
    required: true,
    contributions: &[CHATS],
    settings_section: None,
    settings: SettingsSchema::NONE,
};
