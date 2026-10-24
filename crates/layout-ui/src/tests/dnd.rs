//! Dragging an item of the bar, in the lists or in the preview, edits the layout in use like
//! the keys and the menus do: reordering, moving between zones and groups, to and from the tray.

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
fn a_row_dropped_on_another_row_takes_the_place_in_front_of_it(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-item:shell.terminal",
        "layout-item:shell.transfers",
        cx,
    );

    assert_eq!(
        fixture.top_to_bottom(&RIGHT, cx),
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
fn a_chip_of_the_preview_dropped_on_another_chip_reorders_the_same_way(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.terminal",
        "layout-preview:shell.transfers",
        cx,
    );

    assert_eq!(
        fixture.top_to_bottom(&RIGHT, cx),
        [
            "shell.notices",
            "shell.terminal",
            "shell.transfers",
            "tools.clock"
        ]
    );
}

#[gpui_kit::test]
fn a_widget_dropped_on_another_zone_moves_there(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-item:shell.transfers", "layout-zone:center", cx);

    let moved = x_of(&fixture, "layout-item:shell.transfers", cx);
    assert!(
        moved < x_of(&fixture, "layout-item:shell.notices", cx),
        "transfers left the right zone"
    );
    assert!(
        moved > x_of(&fixture, "layout-item:shell.servers", cx),
        "and is not in the left one"
    );
}

#[gpui_kit::test]
fn a_widget_dropped_on_the_preview_zone_moves_there(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-preview:shell.transfers",
        "layout-preview-zone:center",
        cx,
    );

    assert!(
        x_of(&fixture, "layout-item:shell.transfers", cx)
            < x_of(&fixture, "layout-item:shell.notices", cx)
    );
}

#[gpui_kit::test]
fn a_panel_dropped_on_another_group_joins_it(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-item:shell.files", "layout-item:group:assistant", cx);

    assert_eq!(
        fixture.top_to_bottom(&["group:assistant", "shell.agent", "shell.files"], cx),
        ["group:assistant", "shell.agent", "shell.files"]
    );
    assert_eq!(fixture.stored(cx).presets.len(), 1);
}

#[gpui_kit::test]
fn a_panel_dropped_on_a_panel_of_another_group_lands_before_it(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-item:shell.files", "layout-item:shell.agent", cx);

    assert_eq!(
        fixture.top_to_bottom(&["group:assistant", "shell.files", "shell.agent"], cx),
        ["group:assistant", "shell.files", "shell.agent"]
    );
}

#[gpui_kit::test]
fn a_group_dropped_on_a_zone_moves_whole(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-item:group:navigation", "layout-zone-end:right", cx);

    let group = x_of(&fixture, "layout-item:group:navigation", cx);
    assert_eq!(group, x_of(&fixture, "layout-item:shell.notices", cx));
    assert_eq!(
        x_of(&fixture, "layout-item:shell.servers", cx),
        group,
        "its panels follow it"
    );
}

#[gpui_kit::test]
fn an_item_dropped_in_the_tray_leaves_the_bar(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-item:tools.clock", "layout-tray", cx);

    assert!(!fixture.has("layout-item:tools.clock", cx));
    assert!(fixture.has("layout-tray:tools.clock", cx));
    assert!(!fixture.has("layout-preview:tools.clock", cx));
}

#[gpui_kit::test]
fn a_chip_of_the_preview_dropped_in_the_tray_leaves_the_bar(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-preview:shell.terminal", "layout-tray", cx);

    assert!(fixture.has("layout-tray:shell.terminal", cx));
    assert!(!fixture.has("layout-item:shell.terminal", cx));
}

#[gpui_kit::test]
fn an_item_dragged_out_of_the_tray_returns_in_front_of_the_row_it_is_dropped_on(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    fixture.drag("layout-item:tools.clock", "layout-tray", cx);
    fixture.drag("layout-tray:tools.clock", "layout-item:shell.notices", cx);

    assert!(!fixture.has("layout-tray:tools.clock", cx));
    assert_eq!(
        fixture.top_to_bottom(&RIGHT, cx),
        [
            "tools.clock",
            "shell.notices",
            "shell.transfers",
            "shell.terminal"
        ]
    );
}

#[gpui_kit::test]
fn the_pinned_notice_cannot_be_dropped_in_the_tray_and_the_editor_says_why(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    fixture.drag("layout-item:shell.notices", "layout-tray", cx);

    assert_eq!(fixture.stored(cx), LayoutSettings::default());
    assert!(fixture.has("layout-item:shell.notices", cx));
    assert!(!fixture.has("layout-tray:shell.notices", cx));
    assert!(fixture.has("layout-notice", cx), "the refusal is shown");
}

#[gpui_kit::test]
fn a_drop_is_one_step_of_the_history(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-item:shell.terminal",
        "layout-item:shell.transfers",
        cx,
    );
    fixture.click("layout-undo", cx);

    assert_eq!(fixture.top_to_bottom(&RIGHT, cx), RIGHT);
}

#[gpui_kit::test]
fn nothing_is_written_when_an_item_is_dropped_where_it_already_is(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag(
        "layout-item:shell.transfers",
        "layout-item:shell.transfers",
        cx,
    );
    fixture.drag("layout-item:shell.transfers", "layout-item:tools.clock", cx);

    assert_eq!(fixture.stored(cx), LayoutSettings::default());
}

#[gpui_kit::test]
fn a_built_in_layout_that_is_not_in_use_takes_no_drops(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-choice:swapped", cx);
    fixture.drag("layout-item:shell.terminal", "layout-tray", cx);

    assert_eq!(fixture.stored(cx), LayoutSettings::default());
}
