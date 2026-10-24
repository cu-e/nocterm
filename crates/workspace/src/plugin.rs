//! What the window shell itself contributes: two core widgets and the panel groups.
//!
//! The shell's contributions are not a plugin: they are never disabled and belong to no
//! Plugins page. [`catalog`] builds a catalog that always lists them through [`CORE`] and its
//! groups through [`GROUPS`].

use nocterm_plugin_model::{
    AutoPlace, CatalogError, ContributionId, ContributionKind, GroupId, GroupSpec, Icon, Placement,
    PluginCatalog, SystemContribution, SystemPlugin, Zone,
};

/// The notification bar in the status bar. It can move but never leave.
pub const NOTIFICATIONS: SystemContribution = SystemContribution {
    id: ContributionId::new("workspace.notifications"),
    kind: ContributionKind::Widget,
    title: "Notifications",
    icon: Icon::Bell,
    placement: Placement {
        zone: Zone::Right,
        order: 10,
        group: None,
        auto: AutoPlace::Show,
    },
    panel: None,
    preview: None,
    pinned: true,
};

/// The toggle of the local terminal dock.
pub const LOCAL_TERMINAL: SystemContribution = SystemContribution {
    id: ContributionId::new("workspace.local-terminal"),
    kind: ContributionKind::Widget,
    title: "Local Terminal",
    icon: Icon::SquareTerminal,
    placement: Placement {
        zone: Zone::Right,
        order: 30,
        group: None,
        auto: AutoPlace::Show,
    },
    panel: None,
    preview: None,
    pinned: false,
};

/// Every contribution of the shell.
pub const CORE: &[SystemContribution] = &[NOTIFICATIONS, LOCAL_TERMINAL];

/// The panel groups of the shell, in the order of the bar: the sidebar, then the assistant.
pub const GROUPS: &[GroupSpec] = &[
    GroupSpec {
        id: GroupId::new("navigation"),
        title: "Sidebar",
        primary: true,
    },
    GroupSpec {
        id: GroupId::new("assistant"),
        title: "Assistant",
        primary: false,
    },
];

/// Checks the shell's groups and contributions together with the `core` contributions of other
/// owners and the system `plugins`, as one catalog. Every window is composed with a catalog
/// from here, so the shell's own contributions and groups are always in it.
pub fn catalog(
    core: &[&'static [SystemContribution]],
    plugins: &[&'static SystemPlugin],
) -> Result<PluginCatalog, CatalogError> {
    let core: Vec<_> = std::iter::once(CORE).chain(core.iter().copied()).collect();
    PluginCatalog::new(GROUPS, &core, plugins)
}

#[cfg(test)]
mod tests {
    use nocterm_plugin_model::CatalogError;

    use super::*;
    use crate::test_support::{TEST_CORE, TEST_PLUGIN};

    #[test]
    fn every_catalog_lists_the_shells_contributions_and_groups() {
        let composed = catalog(&[TEST_CORE], &[&TEST_PLUGIN]).expect("a valid catalog");
        assert!(composed.contribution(&NOTIFICATIONS.id).is_some());
        assert!(composed.contribution(&LOCAL_TERMINAL.id).is_some());
        assert_eq!(composed.groups(), GROUPS);
        let alone = catalog(&[], &[]).expect("the shell alone is a valid catalog");
        assert!(alone.contribution(&NOTIFICATIONS.id).is_some());
    }

    #[test]
    fn the_shells_contributions_cannot_be_listed_twice() {
        assert_eq!(
            catalog(&[CORE], &[]).err(),
            Some(CatalogError::Duplicate(NOTIFICATIONS.id.clone()))
        );
    }
}
