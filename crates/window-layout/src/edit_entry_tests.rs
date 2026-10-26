//! Edits of the entries: moving, grouping, separators, the tray and the open panels.

use nocterm_plugin_model::{GroupId, Side, Zone};

use crate::{
    edit_tests::*,
    tests::{NONE, id},
    *,
};

#[test]
fn a_pinned_widget_cannot_go_to_the_tray_and_the_failed_edit_copies_nothing() {
    let mut document = LayoutSettings::default();

    let refused = apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.notices"),
        },
    );

    assert_eq!(refused, Err(EditError::Pinned(id("shell.notices"))));
    assert_eq!(document, LayoutSettings::default());
}

#[test]
fn an_item_in_the_tray_stays_out_and_restore_puts_it_back_by_its_hints() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.transfers"),
        },
    )
    .unwrap();
    apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.files"),
        },
    )
    .unwrap();

    assert_eq!(document.presets[0].tray.len(), 2);
    assert!(!zone(&document, "p1", Zone::Right).contains(&"shell.transfers".to_owned()));
    let left = zone(&document, "p1", Zone::Left);
    assert!(!left[0].contains("shell.files"), "{left:?}");

    apply(
        &mut document,
        EditOp::Restore {
            id: id("shell.transfers"),
        },
    )
    .unwrap();
    apply(
        &mut document,
        EditOp::Restore {
            id: id("shell.files"),
        },
    )
    .unwrap();

    assert!(document.presets[0].tray.is_empty());
    assert_eq!(
        zone(&document, "p1", Zone::Right).last().unwrap(),
        "shell.transfers"
    );
    assert!(zone(&document, "p1", Zone::Left)[0].ends_with("shell.files]"));
}

#[test]
fn unknown_ids_wait_in_the_tray_until_they_are_forgotten() {
    let mut ghost = stored("p1", "One");
    ghost.bar.left = vec![BarItem::Item {
        id: id("ghost.one"),
    }];
    ghost.tray = vec![id("ghost.two")];
    let mut document = settings(Some("p1"), vec![ghost]);

    apply(&mut document, mv("shell.monitor", 0)).unwrap();

    assert_eq!(document.presets[0].tray, [id("ghost.two"), id("ghost.one")]);

    apply(
        &mut document,
        EditOp::Forget {
            id: id("ghost.two"),
        },
    )
    .unwrap();
    assert_eq!(document.presets[0].tray, [id("ghost.one")]);

    let refused = apply(
        &mut document,
        EditOp::Forget {
            id: id("shell.files"),
        },
    );
    assert_eq!(refused, Err(EditError::Installed(id("shell.files"))));
}

#[test]
fn separators_are_inserted_in_a_zone_and_a_group_and_removed_again() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        EditOp::InsertSeparator {
            place: Slot::Zone(Zone::Right),
            index: 1,
        },
    )
    .unwrap();
    apply(
        &mut document,
        EditOp::InsertSeparator {
            place: Slot::Group(GroupId::new("navigation")),
            index: 1,
        },
    )
    .unwrap();

    assert_eq!(zone(&document, "p1", Zone::Right)[1], "|");
    assert_eq!(
        zone(&document, "p1", Zone::Left)[0],
        "[navigation: shell.servers | shell.files shell.snippets tools.list]"
    );

    apply(
        &mut document,
        EditOp::RemoveSeparator {
            place: Slot::Zone(Zone::Right),
            index: 1,
        },
    )
    .unwrap();
    assert_eq!(zone(&document, "p1", Zone::Right), RIGHT);
    let refused = apply(
        &mut document,
        EditOp::RemoveSeparator {
            place: Slot::Zone(Zone::Right),
            index: 1,
        },
    );
    assert_eq!(refused, Err(EditError::NotASeparator));
}

#[test]
fn a_separator_that_would_lead_a_zone_is_not_kept() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        EditOp::InsertSeparator {
            place: Slot::Zone(Zone::Right),
            index: 0,
        },
    )
    .unwrap();

    assert_eq!(zone(&document, "p1", Zone::Right), RIGHT);
}

#[test]
fn widgets_change_zone_and_panels_change_group() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        EditOp::ToZone {
            subject: item("shell.transfers"),
            zone: Zone::Center,
        },
    )
    .unwrap();
    apply(
        &mut document,
        EditOp::ToGroup {
            panel: id("shell.snippets"),
            group: GroupId::new("assistant"),
        },
    )
    .unwrap();

    assert_eq!(zone(&document, "p1", Zone::Center), ["shell.transfers"]);
    assert_eq!(
        zone(&document, "p1", Zone::Right).last().unwrap(),
        "[assistant: shell.agent shell.snippets]"
    );
    apply(
        &mut document,
        EditOp::ToZone {
            subject: item("shell.files"),
            zone: Zone::Left,
        },
    )
    .unwrap();
    assert_eq!(
        zone(&document, "p1", Zone::Left).last().unwrap(),
        "(shell.files)",
        "a panel sent to a zone leaves its group and stands on its own"
    );
}

#[test]
fn a_group_that_lost_its_last_panel_comes_back_with_the_next_panel_moved_to_it() {
    let mut document = LayoutSettings::default();
    apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.agent"),
        },
    )
    .unwrap();
    assert_eq!(zone(&document, "p1", Zone::Right).len(), 4);

    apply(
        &mut document,
        EditOp::ToGroup {
            panel: id("shell.agent"),
            group: GroupId::new("assistant"),
        },
    )
    .unwrap();

    assert_eq!(zone(&document, "p1", Zone::Right), RIGHT);
}

#[test]
fn a_whole_group_moves_to_another_zone_and_keeps_its_panels() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        EditOp::ToZone {
            subject: Subject::Group(GroupId::new("assistant")),
            zone: Zone::Center,
        },
    )
    .unwrap();

    assert_eq!(
        zone(&document, "p1", Zone::Center),
        ["[assistant: shell.agent]"]
    );
}

#[test]
fn moves_stop_at_the_ends_and_never_leave_the_container() {
    let mut document = LayoutSettings::default();

    apply(&mut document, mv("shell.notices", -5)).unwrap();
    assert_eq!(zone(&document, "p1", Zone::Right)[0], "shell.notices");

    apply(&mut document, mv("shell.files", 9)).unwrap();
    assert_eq!(
        zone(&document, "p1", Zone::Left)[0],
        "[navigation: shell.servers shell.snippets tools.list shell.files]"
    );
    apply(&mut document, mv("tools.list", 1)).unwrap();
    let missing = apply(&mut document, mv("ghost.item", 1));
    assert_eq!(missing, Err(EditError::NotPlaced(item("ghost.item"))));
}

#[test]
fn only_one_panel_of_a_group_opens_with_the_layout() {
    let mut document = LayoutSettings::default();

    for panel in ["shell.files", "shell.servers"] {
        apply(
            &mut document,
            EditOp::SetOpen {
                panel: id(panel),
                open: true,
            },
        )
        .unwrap();
    }
    assert_eq!(document.presets[0].open, [id("shell.servers")]);

    apply(
        &mut document,
        EditOp::SetOpen {
            panel: id("shell.servers"),
            open: false,
        },
    )
    .unwrap();
    assert!(document.presets[0].open.is_empty());
    let refused = apply(
        &mut document,
        EditOp::SetOpen {
            panel: id("ghost.panel"),
            open: true,
        },
    );
    assert_eq!(refused, Err(EditError::NotPlaced(item("ghost.panel"))));
}

#[test]
fn a_group_takes_the_side_it_is_given_and_goes_back_to_its_zone() {
    let mut document = LayoutSettings::default();
    let set = |side| EditOp::SetSide {
        group: GroupId::new("navigation"),
        side,
    };

    apply(&mut document, set(Some(Side::Right))).unwrap();
    let layout = document.resolve(&pid("p1"), &catalog(), &NONE);
    assert_eq!(layout.primary_group().unwrap().side(), Side::Right);

    apply(&mut document, set(None)).unwrap();
    let layout = document.resolve(&pid("p1"), &catalog(), &NONE);
    assert_eq!(layout.primary_group().unwrap().side(), Side::Left);
}

#[test]
fn a_widget_of_the_tray_is_placed_in_the_zone_asked_for() {
    let mut document = LayoutSettings::default();
    apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.transfers"),
        },
    )
    .unwrap();

    apply(
        &mut document,
        EditOp::PlaceIn {
            id: id("shell.transfers"),
            zone: Zone::Center,
        },
    )
    .unwrap();

    assert_eq!(zone(&document, "p1", Zone::Center), ["shell.transfers"]);
    assert!(document.presets[0].tray.is_empty());
}

#[test]
fn a_panel_of_the_tray_rejoins_its_group_and_a_missing_group_takes_the_zone_asked_for() {
    let mut document = LayoutSettings::default();
    apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.files"),
        },
    )
    .unwrap();
    apply(
        &mut document,
        EditOp::PlaceIn {
            id: id("shell.files"),
            zone: Zone::Right,
        },
    )
    .unwrap();
    assert!(
        zone(&document, "p1", Zone::Left)[0].contains("shell.files"),
        "the group is in the bar, so the panel joins it where it is"
    );

    apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.agent"),
        },
    )
    .unwrap();
    apply(
        &mut document,
        EditOp::PlaceIn {
            id: id("shell.agent"),
            zone: Zone::Center,
        },
    )
    .unwrap();

    assert_eq!(
        zone(&document, "p1", Zone::Center),
        ["[assistant: shell.agent]"]
    );
    assert!(
        !zone(&document, "p1", Zone::Right)
            .iter()
            .any(|entry| entry.contains("assistant"))
    );
}

#[test]
fn placing_something_that_is_in_the_bar_changes_nothing() {
    let mut document = LayoutSettings::default();
    apply(&mut document, mv("shell.transfers", -1)).unwrap();
    let before = document.clone();

    apply(
        &mut document,
        EditOp::PlaceIn {
            id: id("shell.transfers"),
            zone: Zone::Left,
        },
    )
    .unwrap();

    assert_eq!(document, before);
}

fn move_to(subject: Subject, place: Slot, index: usize) -> EditOp {
    EditOp::MoveTo {
        subject,
        place,
        index,
    }
}

#[test]
fn a_widget_dropped_before_another_takes_its_place_in_the_zone() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        move_to(item("shell.terminal"), Slot::Zone(Zone::Right), 0),
    )
    .unwrap();

    assert_eq!(
        zone(&document, "p1", Zone::Right),
        [
            "shell.terminal",
            "shell.notices",
            "shell.transfers",
            "tools.clock",
            "[assistant: shell.agent]"
        ]
    );
}

#[test]
fn a_widget_dropped_after_the_entry_that_follows_it_counts_the_places_it_leaves() {
    let mut document = LayoutSettings::default();

    // Before the fourth entry of the list as shown (Terminal), so after Clock.
    apply(
        &mut document,
        move_to(item("shell.notices"), Slot::Zone(Zone::Right), 3),
    )
    .unwrap();

    assert_eq!(
        zone(&document, "p1", Zone::Right),
        [
            "shell.transfers",
            "tools.clock",
            "shell.notices",
            "shell.terminal",
            "[assistant: shell.agent]"
        ]
    );
}

#[test]
fn a_widget_dropped_past_the_end_goes_last_and_across_zones() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        move_to(item("shell.transfers"), Slot::Zone(Zone::Center), 9),
    )
    .unwrap();
    apply(
        &mut document,
        move_to(item("shell.terminal"), Slot::Zone(Zone::Center), 0),
    )
    .unwrap();

    assert_eq!(
        zone(&document, "p1", Zone::Center),
        ["shell.terminal", "shell.transfers"]
    );
    assert!(!zone(&document, "p1", Zone::Right).contains(&"shell.terminal".to_owned()));
}

#[test]
fn a_panel_dropped_into_a_group_lands_at_the_place_and_leaves_its_old_group() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        move_to(
            item("shell.agent"),
            Slot::Group(GroupId::new("navigation")),
            1,
        ),
    )
    .unwrap();

    let left = zone(&document, "p1", Zone::Left);
    assert!(
        left[0].starts_with("[navigation: shell.servers shell.agent shell.files"),
        "{left:?}"
    );
    assert!(
        !zone(&document, "p1", Zone::Right)
            .iter()
            .any(|entry| entry.contains("assistant")),
        "the group it left had no panel and went"
    );
}

#[test]
fn a_panel_reorders_inside_its_group() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        move_to(
            item("shell.snippets"),
            Slot::Group(GroupId::new("navigation")),
            0,
        ),
    )
    .unwrap();

    let left = zone(&document, "p1", Zone::Left);
    assert!(
        left[0].starts_with("[navigation: shell.snippets shell.servers shell.files"),
        "{left:?}"
    );
}

#[test]
fn a_group_dropped_in_a_zone_moves_whole() {
    let mut document = LayoutSettings::default();

    apply(
        &mut document,
        move_to(
            Subject::Group(GroupId::new("navigation")),
            Slot::Zone(Zone::Left),
            2,
        ),
    )
    .unwrap();

    let left = zone(&document, "p1", Zone::Left);
    assert_eq!(left[0], "shell.monitor");
    assert!(left[1].starts_with("[navigation"));
}

#[test]
fn a_widget_of_the_tray_dropped_into_a_zone_leaves_the_tray() {
    let mut document = LayoutSettings::default();
    apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.transfers"),
        },
    )
    .unwrap();

    apply(
        &mut document,
        move_to(item("shell.transfers"), Slot::Zone(Zone::Right), 0),
    )
    .unwrap();

    assert!(document.presets[0].tray.is_empty());
    assert_eq!(zone(&document, "p1", Zone::Right)[0], "shell.transfers");
}

#[test]
fn a_drop_that_does_not_fit_the_kind_is_refused_and_copies_nothing() {
    let mut document = LayoutSettings::default();
    let group = || Slot::Group(GroupId::new("navigation"));

    let widget_in_group = apply(&mut document, move_to(item("shell.terminal"), group(), 0));
    let group_in_group = apply(
        &mut document,
        move_to(Subject::Group(GroupId::new("assistant")), group(), 0),
    );

    assert_eq!(
        widget_in_group,
        Err(EditError::WrongKind(item("shell.terminal")))
    );
    assert_eq!(
        group_in_group,
        Err(EditError::WrongKind(Subject::Group(GroupId::new(
            "assistant"
        ))))
    );
    assert_eq!(document, LayoutSettings::default());
}

#[test]
fn a_drop_into_a_group_that_is_not_in_the_bar_is_refused() {
    let mut document = LayoutSettings::default();
    apply(
        &mut document,
        EditOp::RemoveToTray {
            id: id("shell.agent"),
        },
    )
    .unwrap();
    let before = document.clone();

    let refused = apply(
        &mut document,
        move_to(
            item("shell.files"),
            Slot::Group(GroupId::new("assistant")),
            0,
        ),
    );

    assert_eq!(
        refused,
        Err(EditError::NotPlaced(Subject::Group(GroupId::new(
            "assistant"
        ))))
    );
    assert_eq!(document, before);
}
