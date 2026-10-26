//! The menu of an item offers the moves the layout allows and none it refuses.

use std::collections::BTreeSet;

use nocterm_plugin_enablement::PluginEnablement;
use nocterm_plugin_model::{ContributionId, GroupId, Zone};
use nocterm_window_layout::{EditOp, LayoutSettings, Subject};

use super::catalog;
use crate::{
    menu::{Entry, entries},
    snapshot::Snapshot,
};

fn menu_of(subject: &Subject) -> Vec<Entry> {
    let catalog = catalog();
    let snapshot = Snapshot::new(
        &LayoutSettings::default(),
        &PluginEnablement::default(),
        false,
        None,
        &catalog,
    );
    entries(subject, &snapshot, &catalog)
}

fn item(id: &'static str) -> Subject {
    Subject::Contribution(ContributionId::new(id))
}

fn labels(entries: &[Entry]) -> BTreeSet<String> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Do { label, .. } | Entry::Ask { label, .. } | Entry::Submenu { label, .. } => {
                Some(label.clone())
            }
            Entry::Rule => None,
        })
        .collect()
}

fn op(entries: &[Entry], wanted: &str) -> Option<(EditOp, bool)> {
    entries.iter().find_map(|entry| match entry {
        Entry::Do {
            label, op, enabled, ..
        } if label == wanted => Some((op.clone(), *enabled)),
        _ => None,
    })
}

fn submenu<'a>(entries: &'a [Entry], wanted: &str) -> Option<&'a [Entry]> {
    entries.iter().find_map(|entry| match entry {
        Entry::Submenu { label, entries } if label.starts_with(wanted) => Some(entries.as_slice()),
        _ => None,
    })
}

fn group(id: &'static str) -> Subject {
    Subject::Group(GroupId::new(id))
}

#[test]
fn a_widget_offers_the_other_zones_and_hiding() {
    let menu = menu_of(&item("shell.transfers"));

    let labels = labels(&menu);
    assert!(labels.contains("Move to Left"));
    assert!(labels.contains("Move to Centre"));
    assert!(!labels.contains("Move to Right"), "it is there already");
    assert!(labels.contains("Hide from the bar"));
}

#[test]
fn the_pinned_notice_cannot_be_hidden_from_its_menu() {
    let menu = menu_of(&item("shell.notices"));

    assert!(!labels(&menu).contains("Hide from the bar"));
}

#[test]
fn the_first_widget_cannot_move_left_and_the_last_cannot_move_right() {
    let first = menu_of(&item("shell.notices"));
    let last = menu_of(&item("off.item"));

    assert_eq!(
        op(&first, "Move left").map(|(_, enabled)| enabled),
        Some(false)
    );
    assert_eq!(
        op(&first, "Move right").map(|(_, enabled)| enabled),
        Some(true)
    );
    assert_eq!(
        op(&last, "Move right").map(|(_, enabled)| enabled),
        Some(false)
    );
}

#[test]
fn a_panel_of_a_group_can_leave_it_join_another_or_start_its_own() {
    let menu = menu_of(&item("shell.files"));

    let labels = labels(&menu);
    assert!(labels.contains("Take out of group"));
    assert!(labels.contains("Move into group"));
    assert!(labels.contains("New group"));
    assert!(
        !labels.contains("Move to Centre"),
        "its group moves, not the panel"
    );
    assert_eq!(
        op(&menu, "New group").map(|(op, _)| op),
        Some(EditOp::NewGroup {
            panel: ContributionId::new("shell.files")
        })
    );
}

#[test]
fn a_panel_moves_its_group_between_zones_from_the_group_submenu() {
    let menu = menu_of(&item("shell.files"));
    let of_group = submenu(&menu, "Group").expect("the group's own entries");

    let (to_center, enabled) = op(of_group, "Move to Centre").expect("a zone entry");
    assert!(enabled);
    assert_eq!(
        to_center,
        EditOp::ToZone {
            subject: group("navigation"),
            zone: Zone::Center,
        }
    );
}

#[test]
fn every_group_can_be_renamed_and_every_group_but_the_primary_one_ungrouped() {
    let navigation = labels(&menu_of(&group("navigation")));
    let assistant = labels(&menu_of(&group("assistant")));

    assert!(navigation.contains("Rename group\u{2026}"));
    assert!(!navigation.contains("Ungroup"), "the primary group stays");
    assert!(assistant.contains("Rename group\u{2026}"));
    assert!(assistant.contains("Ungroup"));
}

#[test]
fn only_a_group_in_the_centre_chooses_the_side_it_opens_on() {
    let left = labels(&menu_of(&group("navigation")));
    assert!(!left.contains("Opens on"), "a left group opens on the left");
}

#[test]
fn a_panel_can_open_with_the_layout() {
    let menu = menu_of(&item("shell.files"));

    assert_eq!(
        op(&menu, "Open with the layout"),
        Some((
            EditOp::SetOpen {
                panel: ContributionId::new("shell.files"),
                open: true,
            },
            true
        ))
    );
}
