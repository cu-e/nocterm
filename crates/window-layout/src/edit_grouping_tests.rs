//! Panels that leave their group, groups the user makes, breaks up and names.

use nocterm_plugin_model::{GroupId, Side, Zone};

use crate::{
    edit_tests::*,
    tests::{NONE, id},
    *,
};

fn out_to_zone(panel: &'static str, zone: Zone, index: usize) -> EditOp {
    EditOp::MoveTo {
        subject: item(panel),
        place: Slot::Zone(zone),
        index,
    }
}

fn group_with(panel: &'static str, target: &'static str) -> EditOp {
    EditOp::GroupWith {
        panel: id(panel),
        target: id(target),
    }
}

#[test]
fn a_panel_dropped_in_a_zone_leaves_its_group_and_keeps_its_side_when_moved_again() {
    let mut document = LayoutSettings::default();

    apply(&mut document, out_to_zone("shell.files", Zone::Left, 1)).unwrap();
    apply(
        &mut document,
        EditOp::SetSide {
            group: solo_group_id(&id("shell.files")),
            side: Some(Side::Right),
        },
    )
    .unwrap();
    apply(&mut document, out_to_zone("shell.files", Zone::Center, 0)).unwrap();

    let left = zone(&document, "p1", Zone::Left);
    assert!(!left[0].contains("shell.files"), "{left:?}");
    assert_eq!(zone(&document, "p1", Zone::Center), ["(shell.files)"]);
    let layout = document.resolve(&pid("p1"), &catalog(), &NONE);
    assert_eq!(
        layout.group_of(&id("shell.files")).unwrap().side(),
        Side::Right
    );
}

#[test]
fn a_panel_on_its_own_joins_a_group_when_dropped_in_it() {
    let mut document = LayoutSettings::default();
    apply(&mut document, out_to_zone("shell.files", Zone::Center, 0)).unwrap();

    apply(
        &mut document,
        EditOp::MoveTo {
            subject: item("shell.files"),
            place: Slot::Group(GroupId::new("assistant")),
            index: 0,
        },
    )
    .unwrap();

    assert!(zone(&document, "p1", Zone::Center).is_empty());
    assert_eq!(
        zone(&document, "p1", Zone::Right).last().unwrap(),
        "[assistant: shell.files shell.agent]"
    );
}

#[test]
fn a_panel_dropped_on_one_of_its_own_makes_a_named_group_in_its_place() {
    let mut document = LayoutSettings::default();
    apply(&mut document, out_to_zone("shell.files", Zone::Center, 0)).unwrap();

    apply(&mut document, group_with("shell.servers", "shell.files")).unwrap();
    apply(&mut document, group_with("tools.list", "shell.servers")).unwrap();

    assert_eq!(
        zone(&document, "p1", Zone::Center),
        ["[group-1: shell.files shell.servers tools.list]"]
    );
    let layout = document.resolve(&pid("p1"), &catalog(), &NONE);
    let made = layout.group_of(&id("shell.files")).unwrap();
    assert_eq!(made.name(), Some("Group 1"));
    assert_eq!(made.side(), Side::Left);
}

#[test]
fn a_new_group_takes_a_free_id_and_name() {
    let mut document = LayoutSettings::default();
    apply(
        &mut document,
        EditOp::NewGroup {
            panel: id("shell.files"),
        },
    )
    .unwrap();
    apply(
        &mut document,
        EditOp::NewGroup {
            panel: id("tools.list"),
        },
    )
    .unwrap();

    let left = zone(&document, "p1", Zone::Left);
    assert_eq!(left[1], "[group-2: tools.list]", "{left:?}");
    assert_eq!(left[2], "[group-1: shell.files]", "{left:?}");
    let layout = document.resolve(&pid("p1"), &catalog(), &NONE);
    let names: Vec<_> = layout.groups().filter_map(ResolvedGroup::name).collect();
    assert_eq!(names, ["Group 2", "Group 1"]);
}

#[test]
fn breaking_up_a_group_leaves_its_panels_on_their_own_in_its_place() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        EditOp::Ungroup {
            group: GroupId::new("assistant"),
        },
    )
    .unwrap();
    let refused = apply(
        &mut document,
        EditOp::Ungroup {
            group: GroupId::new("navigation"),
        },
    );

    assert_eq!(
        zone(&document, "p1", Zone::Right).last().unwrap(),
        "(shell.agent)"
    );
    assert_eq!(refused, Err(EditError::PrimaryGroup));
}

#[test]
fn a_group_is_renamed_with_a_trimmed_name_that_is_not_empty() {
    let mut document = LayoutSettings::default();
    apply(
        &mut document,
        EditOp::NewGroup {
            panel: id("shell.files"),
        },
    )
    .unwrap();
    let group = GroupId::new("group-1");

    let empty = apply(
        &mut document,
        EditOp::RenameGroup {
            group: group.clone(),
            name: "  ".to_owned(),
        },
    );
    apply(
        &mut document,
        EditOp::RenameGroup {
            group,
            name: " Tools ".to_owned(),
        },
    )
    .unwrap();

    assert_eq!(empty, Err(EditError::Name(NameError::Empty)));
    let layout = document.resolve(&pid("p1"), &catalog(), &NONE);
    let named = layout.group_of(&id("shell.files")).unwrap();
    assert_eq!(named.name(), Some("Tools"));
}

#[test]
fn a_panel_of_its_own_opens_with_the_layout_beside_the_open_panel_of_a_group() {
    let mut document = LayoutSettings::default();
    apply(&mut document, out_to_zone("shell.files", Zone::Left, 1)).unwrap();

    for panel in ["shell.servers", "shell.files"] {
        apply(
            &mut document,
            EditOp::SetOpen {
                panel: id(panel),
                open: true,
            },
        )
        .unwrap();
    }

    let layout = document.resolve(&pid("p1"), &catalog(), &NONE);
    assert_eq!(layout.open(), [id("shell.servers"), id("shell.files")]);
}
