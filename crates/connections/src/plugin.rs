//! What the connections feature contributes to the window shell.
//!
//! Connections are core: the saved servers are never disabled and belong to no Plugins page.
//! A catalog lists them through [`CORE`].
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, PanelChrome, PanelTraits, Placement,
    SystemContribution, WidthClass, Zone,
};

/// The saved servers in the sidebar.
pub const SERVERS: SystemContribution = SystemContribution {
    id: ContributionId::new("connections.servers"),
    kind: ContributionKind::Panel,
    title: "Connections",
    icon: Icon::Server,
    placement: Placement {
        zone: Zone::Left,
        order: 10,
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

/// Every contribution of the feature.
pub const CORE: &[SystemContribution] = &[SERVERS];

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;
