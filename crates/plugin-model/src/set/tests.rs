use super::*;
use crate::{
    AutoPlace, ContributionKind, GroupId, Icon, PanelChrome, PanelTraits, Placement, PluginTier,
    SettingsSchema, SystemContribution, SystemPlugin, UserContribution, WidthClass, Zone,
};
use alloc::{sync::Arc, vec, vec::Vec};

static CLOCK: SystemPlugin = SystemPlugin {
    id: PluginId::new("clock"),
    name: "Clock",
    description: "A clock",
    required: false,
    contributions: &[SystemContribution {
        id: ContributionId::new("clock.time"),
        kind: ContributionKind::Widget,
        title: "Time",
        icon: Icon::Clock,
        placement: Placement {
            zone: Zone::Right,
            order: 1,
            group: None,
            auto: AutoPlace::Show,
        },
        panel: None,
        preview: Some("12:00"),
        pinned: false,
    }],
    settings_section: None,
    settings: SettingsSchema::NONE,
    settings_page: None,
};

fn catalog() -> Arc<PluginCatalog> {
    let groups = [GroupSpec {
        id: GroupId::new("navigation"),
        title: "Sidebar",
        primary: true,
    }];
    Arc::new(PluginCatalog::new(&groups, &[], &[&CLOCK]).unwrap())
}

fn user(publisher: &str, name: &str, enabled: bool, placeable: bool) -> UserEntry {
    let id = PluginId::user(publisher, name).unwrap();
    let panel = UserContribution::clamp(
        ContributionId::in_plugin(&id, "notes").unwrap(),
        ContributionKind::Panel,
        "Notes",
        Icon::Puzzle,
        Zone::Left,
        1,
    )
    .unwrap();
    UserEntry {
        plugin: UserPlugin::new(id, name, "A plugin", vec![panel], SettingsSchema::NONE).unwrap(),
        enabled,
        placeable,
    }
}

fn set(users: Vec<UserEntry>) -> PluginSet {
    PluginSet::system(catalog()).with_users(users, 3).unwrap()
}

#[test]
fn both_tiers_are_listed_and_found_by_id() {
    let set = set(vec![user("acme", "notes", true, true)]);
    let ids: Vec<_> = set.entries().map(|entry| entry.id().as_str()).collect();
    assert_eq!(ids, ["clock.time", "user-acme-notes.notes"]);
    let plugins: Vec<_> = set.plugins().map(|plugin| plugin.tier()).collect();
    assert_eq!(plugins, [PluginTier::System, PluginTier::User]);
    let panel = ContributionId::new("user-acme-notes.notes");
    assert_eq!(set.contribution(&panel).map(|c| c.title()), Some("Notes"));
    assert_eq!(
        set.plugin_of(&panel).map(|plugin| plugin.name()),
        Some("notes")
    );
    assert!(matches!(
        set.owner(&panel),
        Some(SetOwner::Plugin(PluginRef::User(_)))
    ));
    assert_eq!(set.revision(), 3);
}

#[test]
fn a_user_panel_asks_for_a_navigation_column_and_a_shell_title() {
    let set = set(vec![user("acme", "notes", true, true)]);
    let panel = set
        .contribution(&ContributionId::new("user-acme-notes.notes"))
        .unwrap();
    assert_eq!(
        panel.panel(),
        Some(PanelTraits {
            width: WidthClass::Navigation,
            chrome: PanelChrome::Titled,
        })
    );
    assert!(panel.is_user() && !panel.pinned() && panel.preview().is_none());
}

#[test]
fn only_approved_user_contributions_and_every_system_one_are_placeable() {
    let set = set(vec![
        user("acme", "notes", true, true),
        user("acme", "later", true, false),
    ]);
    assert!(set.is_placeable(&ContributionId::new("clock.time")));
    assert!(set.is_placeable(&ContributionId::new("user-acme-notes.notes")));
    assert!(!set.is_placeable(&ContributionId::new("user-acme-later.notes")));
    assert!(!set.is_placeable(&ContributionId::new("user-acme-gone.notes")));
}

#[test]
fn switched_off_user_plugins_join_the_disabled_set() {
    let set = set(vec![
        user("acme", "notes", true, true),
        user("acme", "later", false, true),
    ]);
    let mut disabled = BTreeSet::new();
    set.extend_disabled(&mut disabled);
    let names: Vec<_> = disabled.iter().map(PluginId::as_str).collect();
    assert_eq!(names, ["user-acme-later"]);
}

#[test]
fn a_plugin_installed_twice_is_refused() {
    let error = PluginSet::system(catalog())
        .with_users(
            vec![
                user("acme", "notes", true, true),
                user("acme", "notes", true, true),
            ],
            1,
        )
        .err();
    assert_eq!(
        error,
        Some(PluginSetError::Duplicate(
            PluginId::user("acme", "notes").unwrap()
        ))
    );
}

#[test]
fn only_a_held_set_keeps_ids_of_the_user_tier_it_cannot_show() {
    let user_id = ContributionId::new("user-acme-notes.notes");
    let system_id = ContributionId::new("clock.time");

    let installed = PluginSet::system(catalog());
    assert_eq!(installed.availability(), UserAvailability::Installed);
    assert!(!installed.keeps_unavailable(&user_id));

    let held = installed.clone().with_availability(UserAvailability::Held);
    assert!(held.keeps_unavailable(&user_id));
    assert!(!held.keeps_unavailable(&system_id));

    let developer = installed.with_availability(UserAvailability::Developer);
    assert!(!developer.keeps_unavailable(&user_id));
}

#[test]
fn installing_users_into_a_set_keeps_the_mode_of_the_run() {
    let developer = PluginSet::system(catalog()).with_availability(UserAvailability::Developer);
    let with_users = developer
        .with_users(vec![user("acme", "notes", true, true)], 4)
        .unwrap();
    assert_eq!(with_users.availability(), UserAvailability::Developer);
}

#[test]
fn a_developer_session_names_user_contributions_with_dev_and_other_runs_do_not() {
    let installed = PluginSet::system(catalog());
    let developer = installed
        .clone()
        .with_availability(UserAvailability::Developer);
    let held = installed.clone().with_availability(UserAvailability::Held);

    assert_eq!(
        installed.user_label("Notes", "Board"),
        "Plugin \u{b7} Notes \u{2014} Board"
    );
    assert_eq!(
        held.user_label("Notes", "Board"),
        installed.user_label("Notes", "Board")
    );
    assert_eq!(
        developer.user_label("Notes", "Board"),
        "Plugin \u{b7} Notes \u{2014} Board \u{b7} DEV"
    );
}
