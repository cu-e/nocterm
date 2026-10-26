//! Dragging an item of the bar preview edits the layout in use like the keys and the menus do:
//! reordering, moving between zones and groups, to and from the tray.

use gpui_kit::{TestAppContext, test::TestWindowExt as _};
use nocterm_window_layout::LayoutSettings;

use super::{Fixture, RIGHT, open};

fn x_of(fixture: &Fixture, id: &'static str, cx: &mut TestAppContext) -> f32 {
    let mut x = None;
    fixture.frame(cx, |window, _| {
        x = Some(f32::from(window.find(id).bounds().origin.x));
    });
    x.unwrap_or_else(|| panic!("{id} is not in the frame"))
}

#[gpui_kit::test]
fn a_chip_dropped_on_another_chip_takes_the_place_in_front_of_it(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.terminal",
        "layout-preview:shell.transfers",
        cx,
    );

    assert_eq!(
        fixture.left_to_right(&RIGHT, cx),
        [
            "shell.notices",
            "shell.terminal",
            "shell.transfers",
            "tools.clock"
        ]
    );
    let stored = fixture.stored(cx);
    assert_eq!(
        stored.presets.len(),
        1,
        "the first edit saved a custom layout"
    );
}

#[gpui_kit::test]
fn a_chip_dropped_on_a_gap_takes_that_place(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.terminal",
        "layout-preview-gap:right:0",
        cx,
    );

    assert_eq!(
        fixture.left_to_right(&RIGHT, cx),
        [
            "shell.terminal",
            "shell.notices",
            "shell.transfers",
            "tools.clock"
        ]
    );
}

#[gpui_kit::test]
fn a_widget_dropped_on_another_zone_moves_there(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.transfers",
        "layout-preview-zone:center",
        cx,
    );

    let moved = x_of(&fixture, "layout-preview:shell.transfers", cx);
    assert!(
        moved < x_of(&fixture, "layout-preview:shell.notices", cx),
        "transfers left the right zone"
    );
    assert!(
        moved > x_of(&fixture, "layout-preview:shell.files", cx),
        "and is not in the left one"
    );
}

#[gpui_kit::test]
fn a_panel_dropped_on_another_group_joins_it_at_the_end(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.files",
        "layout-preview-member-gap:assistant:1",
        cx,
    );

    assert_eq!(
        fixture.left_to_right(&["shell.files", "shell.agent", "shell.servers"], cx),
        ["shell.servers", "shell.agent", "shell.files"]
    );
    assert_eq!(fixture.stored(cx).presets.len(), 1);
}

#[gpui_kit::test]
fn a_panel_dropped_on_a_panel_of_another_group_lands_in_front_of_it(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.files",
        "layout-preview:shell.agent",
        cx,
    );

    assert_eq!(
        fixture.left_to_right(&["shell.files", "shell.agent"], cx),
        ["shell.files", "shell.agent"]
    );
    assert!(
        x_of(&fixture, "layout-preview:shell.files", cx)
            > x_of(&fixture, "layout-preview-group:assistant", cx),
        "it is in the assistant's group"
    );
}

#[gpui_kit::test]
fn a_group_dropped_on_a_zone_moves_whole(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview-group:navigation",
        "layout-preview-gap:right:0",
        cx,
    );

    let group = x_of(&fixture, "layout-preview-group:navigation", cx);
    assert!(group < x_of(&fixture, "layout-preview:shell.notices", cx));
    assert!(group > x_of(&fixture, "layout-preview-zone:center", cx));
    assert!(
        x_of(&fixture, "layout-preview:shell.servers", cx) > group,
        "its panels follow it"
    );
}

#[gpui_kit::test]
fn a_chip_dropped_in_the_tray_leaves_the_bar(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-preview:shell.terminal", "layout-tray", cx);

    assert!(fixture.has("layout-tray:shell.terminal", cx));
    assert!(!fixture.has("layout-preview:shell.terminal", cx));
}

#[gpui_kit::test]
fn an_item_dragged_out_of_the_tray_returns_in_front_of_the_chip_it_is_dropped_on(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    fixture.drag("layout-preview:tools.clock", "layout-tray", cx);
    fixture.drag(
        "layout-tray:tools.clock",
        "layout-preview:shell.notices",
        cx,
    );

    assert!(!fixture.has("layout-tray:tools.clock", cx));
    assert_eq!(
        fixture.left_to_right(&RIGHT, cx),
        [
            "tools.clock",
            "shell.notices",
            "shell.transfers",
            "shell.terminal"
        ]
    );
}

#[gpui_kit::test]
fn a_click_on_an_item_of_the_tray_puts_it_back(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-preview:tools.clock", "layout-tray", cx);
    fixture.click("layout-tray:tools.clock", cx);

    assert!(fixture.has("layout-preview:tools.clock", cx));
    assert!(!fixture.has("layout-tray:tools.clock", cx));
}

#[gpui_kit::test]
fn the_pinned_notice_cannot_be_dropped_in_the_tray_and_the_editor_says_why(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    fixture.drag("layout-preview:shell.notices", "layout-tray", cx);

    assert_eq!(fixture.stored(cx), LayoutSettings::default());
    assert!(fixture.has("layout-preview:shell.notices", cx));
    assert!(!fixture.has("layout-tray:shell.notices", cx));
    assert!(fixture.has("layout-notice", cx), "the refusal is shown");
}

#[gpui_kit::test]
fn a_drop_is_one_step_of_the_history(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.terminal",
        "layout-preview:shell.transfers",
        cx,
    );
    fixture.click("layout-undo", cx);

    assert_eq!(fixture.left_to_right(&RIGHT, cx), RIGHT);
}

#[gpui_kit::test]
fn nothing_is_written_when_an_item_is_dropped_where_it_already_is(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.transfers",
        "layout-preview:shell.transfers",
        cx,
    );
    fixture.drag(
        "layout-preview:shell.transfers",
        "layout-preview:tools.clock",
        cx,
    );

    assert_eq!(fixture.stored(cx), LayoutSettings::default());
}

#[gpui_kit::test]
fn a_built_in_layout_that_is_not_in_use_takes_no_drops(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-choice:swapped", cx);
    fixture.drag("layout-preview:shell.terminal", "layout-tray", cx);

    assert_eq!(fixture.stored(cx), LayoutSettings::default());
}
