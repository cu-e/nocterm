//! What the files feature contributes to the window shell.
//!
//! Files are core: the Explorer and the transfer status are never disabled and belong to no
//! Plugins page. A catalog lists them through [`CORE`].
use nocterm_plugin_model::{
    AutoPlace, ContributionId, ContributionKind, Icon, PanelChrome, PanelTraits, Placement,
    SystemContribution, WidthClass, Zone,
};

/// The Explorer in the sidebar.
pub const EXPLORER: SystemContribution = SystemContribution {
    id: ContributionId::new("files.explorer"),
    kind: ContributionKind::Panel,
    title: "Explorer",
    icon: Icon::FolderTree,
    placement: Placement {
        zone: Zone::Left,
        order: 20,
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

/// The progress of file transfers in the status bar.
pub const TRANSFERS: SystemContribution = SystemContribution {
    id: ContributionId::new("files.transfers"),
    kind: ContributionKind::Widget,
    title: "Transfers",
    icon: Icon::Upload,
    placement: Placement {
        zone: Zone::Right,
        order: 20,
        group: None,
        auto: AutoPlace::Show,
    },
    panel: None,
    preview: None,
    pinned: false,
};

/// Every contribution of the feature.
pub const CORE: &[SystemContribution] = &[EXPLORER, TRANSFERS];
