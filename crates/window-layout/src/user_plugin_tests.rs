//! User plugin contributions in a resolved layout: placed only once the user approved the
//! package, kept in the preset, and never placed by a hint.

use std::collections::BTreeSet;

use nocterm_plugin_model::{
    ContributionKind, GroupId, Icon, PluginId, PluginSet, SettingsSchema, UserAvailability,
    UserContribution, UserEntry, UserPlugin, Zone,
};

use crate::{
    edit_tests::stored,
    tests::{NONE, id},
    *,
};

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
        name: None,
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

/// A layout that names a user widget and a user panel, as a user left it before a held start.
fn placed_before_a_held_start(name: &str) -> LayoutSettings {
    let mut preset = wanting_both(name);
    preset.bar.right.insert(
        0,
        BarItem::Item {
            id: id("tools.clock"),
        },
    );
    crate::edit_tests::settings(Some("p1"), vec![preset])
}

fn held() -> PluginSet {
    crate::edit_tests::catalog().with_availability(UserAvailability::Held)
}

fn edit_clock(document: &mut LayoutSettings, catalog: &PluginSet) {
    edit(
        document,
        false,
        catalog,
        EditTarget::Selected,
        EditOp::Move {
            subject: Subject::Contribution(id("tools.clock")),
            by: 1,
        },
    )
    .expect("the clock is placed");
}

fn bar_names(document: &LayoutSettings, zone: Zone) -> Vec<BarItem> {
    document.presets[0].bar.zone(zone).to_vec()
}

#[test]
fn a_layout_edit_in_a_run_that_holds_user_plugins_leaves_their_entries_where_they_are() {
    let mut document = placed_before_a_held_start("notes");

    edit_clock(&mut document, &held());

    let preset = &document.presets[0];
    let chip = BarItem::Item {
        id: id_of("notes", "chip"),
    };
    assert_eq!(bar_names(&document, Zone::Right).last(), Some(&chip));
    let navigation = preset
        .bar
        .zone(Zone::Left)
        .iter()
        .find_map(|item| match item {
            BarItem::Group { id, members, .. } if id.as_str() == "navigation" => Some(members),
            _ => None,
        })
        .expect("the navigation group stays");
    assert_eq!(
        navigation.last(),
        Some(&GroupEntry::Panel(id_of("notes", "view")))
    );
    assert!(preset.tray.is_empty(), "{:?}", preset.tray);
}

#[test]
fn the_same_edit_in_a_run_that_knows_no_such_plugin_moves_the_entries_to_the_tray() {
    let mut document = placed_before_a_held_start("notes");

    edit_clock(&mut document, &crate::edit_tests::catalog());

    let preset = &document.presets[0];
    assert!(preset.tray.contains(&id_of("notes", "chip")));
    assert!(preset.tray.contains(&id_of("notes", "view")));
    assert!(!ids_in_bar(&document).contains(&id_of("notes", "chip")));
}

#[test]
fn entries_a_held_run_kept_come_back_in_place_once_the_plugin_is_installed_again() {
    let mut document = placed_before_a_held_start("notes");
    edit_clock(&mut document, &held());

    let installed = set(vec![entry("notes", true, true)]);
    let layout = document.resolve(&crate::edit_tests::pid("p1"), &installed, &NONE);

    assert!(layout.is_placed(&id_of("notes", "chip")));
    assert!(layout.is_placed(&id_of("notes", "view")));
}

#[test]
fn a_held_edit_counts_positions_over_the_entries_the_editor_shows() {
    let mut document = placed_before_a_held_start("notes");
    edit_clock(&mut document, &held());
    let before = zone_of_visible(&document);

    edit(
        &mut document,
        false,
        &held(),
        EditTarget::Selected,
        EditOp::MoveTo {
            subject: Subject::Contribution(id("tools.clock")),
            place: Slot::Zone(Zone::Right),
            index: 0,
        },
    )
    .expect("the clock moves");

    let after = zone_of_visible(&document);
    assert_eq!(after.first().map(String::as_str), Some("tools.clock"));
    assert_eq!(after.len(), before.len());
    assert!(ids_in_bar(&document).contains(&id_of("notes", "chip")));
}

fn zone_of_visible(document: &LayoutSettings) -> Vec<String> {
    let layout = document.resolve(&crate::edit_tests::pid("p1"), &held(), &NONE);
    crate::tests::show(layout.zone(Zone::Right))
}

fn ids_in_bar(document: &LayoutSettings) -> Vec<nocterm_plugin_model::ContributionId> {
    let bar = &document.presets[0].bar;
    [Zone::Left, Zone::Center, Zone::Right]
        .into_iter()
        .flat_map(|zone| bar.zone(zone).to_vec())
        .flat_map(|item| match item {
            BarItem::Item { id } | BarItem::Panel { id, .. } => vec![id],
            BarItem::Group { members, .. } => members
                .into_iter()
                .filter_map(|member| match member {
                    GroupEntry::Panel(id) => Some(id),
                    GroupEntry::Separator => None,
                })
                .collect(),
            BarItem::Separator => Vec::new(),
        })
        .collect()
}
