//! User plugin contributions in a resolved layout: placed only once the user approved the
//! package, kept in the preset, and never placed by a hint.

use std::collections::BTreeSet;

use nocterm_plugin_model::{
    ContributionKind, GroupId, Icon, PluginId, PluginSet, SettingsSchema, UserContribution,
    UserEntry, UserPlugin, Zone,
};

use crate::{edit_tests::stored, tests::NONE, *};

fn entry(name: &str, enabled: bool, placeable: bool) -> UserEntry {
    let plugin = PluginId::user("acme", name).unwrap();
    let local = |local, kind, zone| {
        UserContribution::clamp(
            nocterm_plugin_model::ContributionId::in_plugin(&plugin, local).unwrap(),
            kind,
            "Title",
            Icon::Puzzle,
            zone,
            1,
        )
        .unwrap()
    };
    let contributions = vec![
        local("view", ContributionKind::Panel, Zone::Left),
        local("chip", ContributionKind::Widget, Zone::Right),
    ];
    UserEntry {
        plugin: UserPlugin::new(plugin, name, "", contributions, SettingsSchema::NONE).unwrap(),
        enabled,
        placeable,
    }
}

fn set(users: Vec<UserEntry>) -> PluginSet {
    crate::edit_tests::catalog()
        .with_users(users, 1)
        .expect("distinct plugins")
}

fn wanting_both(name: &str) -> LayoutPreset {
    let mut preset = stored("p1", "A");
    preset.bar.right = vec![BarItem::Item {
        id: id_of(name, "chip"),
    }];
    preset.bar.left = vec![BarItem::Group {
        id: GroupId::new("navigation"),
        side: None,
        primary: true,
        members: vec![GroupEntry::Panel(id_of(name, "view"))],
    }];
    preset
}

fn id_of(name: &str, local: &str) -> nocterm_plugin_model::ContributionId {
    nocterm_plugin_model::ContributionId::owned(format!("user-acme-{name}.{local}"))
}

#[test]
fn a_placeable_user_plugin_is_placed_where_the_preset_names_it() {
    let set = set(vec![entry("notes", true, true)]);

    let layout = resolve(&wanting_both("notes"), &set, &NONE);

    assert!(layout.is_placed(&id_of("notes", "chip")));
    assert!(layout.is_placed(&id_of("notes", "view")));
}

#[test]
fn a_user_plugin_without_approval_is_skipped_and_left_out_of_the_tray() {
    let set = set(vec![entry("notes", true, false)]);

    let layout = resolve(&wanting_both("notes"), &set, &NONE);

    assert!(!layout.is_placed(&id_of("notes", "chip")));
    assert!(!layout.is_placed(&id_of("notes", "view")));
    assert!(layout.host_of(&id_of("notes", "view")).is_none());
}

#[test]
fn a_user_plugin_waits_in_the_tray_until_the_user_places_it() {
    let set = set(vec![entry("notes", true, true)]);

    let layout = BuiltIn::Default.resolve(&set, &NONE);

    assert!(!layout.is_placed(&id_of("notes", "view")));
    assert!(!layout.is_placed(&id_of("notes", "chip")));
    assert_eq!(
        layout.host_of(&id_of("notes", "view")).map(GroupId::as_str),
        Some("navigation")
    );
}

#[test]
fn a_disabled_user_plugin_keeps_its_place_in_the_preset_but_is_not_drawn() {
    let set = set(vec![entry("notes", false, true)]);
    let mut disabled = BTreeSet::new();
    set.extend_disabled(&mut disabled);

    let layout = resolve(&wanting_both("notes"), &set, &disabled);

    assert!(!layout.is_placed(&id_of("notes", "chip")));
    assert_eq!(layout.disabled().len(), 2);
}

#[test]
fn the_editor_cannot_place_a_user_contribution_that_waits_for_approval() {
    let mut document = LayoutSettings::default();
    let waiting = set(vec![entry("notes", true, false)]);

    let refused = edit(
        &mut document,
        false,
        &waiting,
        EditTarget::Selected,
        EditOp::Restore {
            id: id_of("notes", "chip"),
        },
    );

    assert!(matches!(refused, Err(EditError::Unknown(_))), "{refused:?}");
    assert_eq!(document, LayoutSettings::default());
}
