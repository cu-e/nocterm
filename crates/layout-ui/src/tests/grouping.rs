//! Panels stand alone in the bar or share a group: dragging and the menus take a panel out of
//! its group, put two panels together, start, rename and undo a group.

use gpui_kit::{Bounds, Pixels, TestAppContext, test::TestWindowExt as _};

use super::{Fixture, open, preview_id};

fn bounds(fixture: &Fixture, id: &str, cx: &mut TestAppContext) -> Bounds<Pixels> {
    let mut found = None;
    fixture.frame(cx, |window, _| {
        found = window
            .try_find(preview_id(id))
            .map(|element| element.bounds());
    });
    found.unwrap_or_else(|| panic!("{id} is not in the frame"))
}

/// Whether the chip of `panel` is drawn inside the pill of `group`.
fn in_group(fixture: &Fixture, panel: &str, group: &str, cx: &mut TestAppContext) -> bool {
    let (chip, pill) = (
        bounds(fixture, panel, cx),
        bounds(fixture, &format!("group:{group}"), cx),
    );
    chip.left() >= pill.left() && chip.right() <= pill.right()
}

#[gpui_kit::test]
fn a_panel_dropped_between_groups_stands_alone_there(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.files",
        "layout-preview-gap:left:1",
        cx,
    );

    assert!(!in_group(&fixture, "shell.files", "navigation", cx));
    assert!(!in_group(&fixture, "shell.files", "assistant", cx));
    assert_eq!(
        fixture.left_to_right(&["group:navigation", "shell.files", "group:assistant"], cx),
        ["group:navigation", "shell.files", "group:assistant"]
    );
    assert_eq!(fixture.stored(cx).presets.len(), 1);
}

#[gpui_kit::test]
fn a_panel_taken_out_of_its_group_from_the_menu_stands_right_after_it(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose("layout-preview:shell.files", "Take out of group", cx);

    assert!(!in_group(&fixture, "shell.files", "navigation", cx));
    assert!(in_group(&fixture, "shell.servers", "navigation", cx));
    assert_eq!(
        fixture.left_to_right(&["group:navigation", "shell.files", "group:assistant"], cx),
        ["group:navigation", "shell.files", "group:assistant"]
    );
}

#[gpui_kit::test]
fn a_panel_dropped_on_a_panel_that_stands_alone_makes_a_group_of_the_two(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose("layout-preview:shell.files", "Take out of group", cx);
    fixture.drag(
        "layout-preview:shell.agent",
        "layout-preview:shell.files",
        cx,
    );

    assert!(fixture.has("layout-preview-group:group-1", cx));
    assert!(in_group(&fixture, "shell.files", "group-1", cx));
    assert!(in_group(&fixture, "shell.agent", "group-1", cx));
}

#[gpui_kit::test]
fn a_new_group_from_the_menu_holds_the_panel_and_ungrouping_lets_it_stand_alone(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    fixture.choose("layout-preview:shell.agent", "New group", cx);

    assert!(in_group(&fixture, "shell.agent", "group-1", cx));

    fixture.choose("layout-preview-member-gap:group-1:0", "Ungroup", cx);
    assert!(!fixture.has("layout-preview-group:group-1", cx));
    assert!(fixture.has("layout-preview:shell.agent", cx));
}

#[gpui_kit::test]
fn a_group_is_renamed_from_its_menu(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose(
        "layout-preview-member-gap:assistant:0",
        "Rename group\u{2026}",
        cx,
    );
    assert!(fixture.has("layout-rename-row", cx));

    fixture.keys("ctrl-a", cx);
    fixture.keys("H e l p enter", cx);

    assert!(!fixture.has("layout-rename-row", cx));
    assert_eq!(
        fixture
            .label("layout-preview-group:assistant", cx)
            .as_deref(),
        Some("Help")
    );
}

#[gpui_kit::test]
fn a_click_on_a_panel_of_a_group_selects_the_panel_and_one_beside_it_the_group(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.agent", cx);

    assert!(fixture.is_selected("shell.agent", cx));
    assert!(!fixture.is_selected("group:assistant", cx));

    fixture.click("layout-preview-member-gap:assistant:0", cx);
    assert!(fixture.is_selected("group:assistant", cx));
    assert!(!fixture.is_selected("shell.agent", cx));
}

#[gpui_kit::test]
fn a_right_click_on_a_panel_of_a_group_opens_the_menu_of_the_panel(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose("layout-preview:shell.agent", "New group", cx);

    assert!(in_group(&fixture, "shell.agent", "group-1", cx));
}
