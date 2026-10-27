//! What the hubs feature contributes to the window shell.
//!
//! The sessions panel is core: it exists in the window of a hub only, is never disabled and
//! belongs to no Plugins page. A catalog lists it through [`CORE`].
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, PanelChrome, PanelTraits, Placement,
    SystemContribution, WidthClass, Zone,
};

/// The terminal sessions of the hub in the sidebar.
pub const SESSIONS: SystemContribution = SystemContribution {
    id: ContributionId::new("hubs.sessions"),
    kind: ContributionKind::Panel,
    title: "Sessions",
    icon: Icon::SquareTerminal,
    placement: Placement {
        zone: Zone::Left,
        order: 5,
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
pub const CORE: &[SystemContribution] = &[SESSIONS];

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;
