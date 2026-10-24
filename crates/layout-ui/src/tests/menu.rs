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
            Entry::Do { label, .. } | Entry::Submenu { label, .. } => Some(label.clone()),
            Entry::Rule => None,
        })
        .collect()
}

fn op(entries: &[Entry], wanted: &str) -> Option<(EditOp, bool)> {
    entries.iter().find_map(|entry| match entry {
        Entry::Do { label, op, enabled } if label == wanted => Some((op.clone(), *enabled)),
        _ => None,
    })
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
fn the_first_widget_cannot_move_up_and_the_last_cannot_move_down() {
    let first = menu_of(&item("shell.notices"));
    let last = menu_of(&item("off.item"));

    assert_eq!(
        op(&first, "Move up").map(|(_, enabled)| enabled),
        Some(false)
    );
    assert_eq!(
        op(&first, "Move down").map(|(_, enabled)| enabled),
        Some(true)
    );
    assert_eq!(
        op(&last, "Move down").map(|(_, enabled)| enabled),
        Some(false)
    );
}

#[test]
fn a_panel_moves_its_group_between_zones_and_into_another_group() {
    let menu = menu_of(&item("shell.files"));

    let (to_center, enabled) = op(&menu, "Move group to Centre").expect("a zone entry");
    assert!(enabled);
    assert_eq!(
        to_center,
        EditOp::ToZone {
            subject: Subject::Group(GroupId::new("navigation")),
            zone: Zone::Center,
        }
    );
    assert!(labels(&menu).contains("Move into group"));
}
