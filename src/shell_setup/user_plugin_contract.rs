//! The user plugin API and the system catalog must not share names.
use nocterm_plugin_api::{CAPABILITIES, PluginIcon};
use nocterm_plugin_model::Icon;

use super::plugin_catalog;

#[test]
fn no_capability_id_names_a_system_contribution_or_plugin() {
    let catalog = plugin_catalog().expect("the application catalog is valid");
    let contributions = catalog.entries().map(|entry| entry.id.as_str());
    let plugins = catalog.plugins().map(|plugin| plugin.id.as_str());
    let taken: Vec<&str> = contributions.chain(plugins).collect();

    assert!(!taken.is_empty());
    for capability in CAPABILITIES {
        assert!(
            !taken.contains(&capability.id),
            "capability `{}` is also a system id",
            capability.id
        );
    }
}

#[test]
fn no_system_contribution_draws_an_icon_a_user_plugin_may_show() {
    let catalog = plugin_catalog().expect("the application catalog is valid");
    let system: Vec<Icon> = catalog.entries().map(|entry| entry.icon).collect();
    let offered: Vec<Icon> = PluginIcon::ALL.map(PluginIcon::to_icon).to_vec();

    assert!(!system.is_empty() && !offered.is_empty());
    for icon in offered {
        assert!(icon.user_allowed(), "{icon:?} is offered but not allowed");
        assert!(
            !system.contains(&icon),
            "{icon:?} is offered to user plugins and also drawn by a system contribution"
        );
    }
    for icon in Icon::ALL.into_iter().filter(|icon| icon.user_allowed()) {
        assert!(
            PluginIcon::ALL.map(PluginIcon::to_icon).contains(&icon),
            "{icon:?} is allowed but plugins cannot name it"
        );
    }
}
