//! While an item is held over the bar preview, one mark shows where it would land and the ghost
//! looks like the item; a release anywhere ends the drag, and the frame after it shows neither.

use gpui_kit::{Bounds, Pixels, TestAppContext, point, px};
use nocterm_window_layout::LayoutSettings;

use super::{Fixture, open};

const MARKER: &str = "layout-drop-marker";
const GHOST: &str = "layout-drag-ghost";

fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    inner.left() >= outer.left()
        && inner.right() <= outer.right()
        && inner.top() >= outer.top()
        && inner.bottom() <= outer.bottom()
}

/// The point `across` of the way over the element `id`, on its middle line.
fn across(
    fixture: &Fixture,
    id: &str,
    across: f32,
    cx: &mut TestAppContext,
) -> gpui_kit::Point<Pixels> {
    let bounds = fixture.bounds(id, cx);
    point(
        bounds.left() + bounds.size.width * across,
        bounds.center().y,
    )
}

/// Whether the drag ended and the frame drawn after it shows no ghost and no mark.
fn ended(fixture: &Fixture, cx: &mut TestAppContext) -> bool {
    !cx.update(|cx| cx.has_active_drag())
        && fixture.shown(GHOST, cx).is_none()
        && fixture.shown(MARKER, cx).is_none()
}

#[gpui_kit::test]
fn an_item_held_over_a_chip_shows_one_line_where_it_would_land(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let at = across(&fixture, "layout-preview:shell.transfers", 0.25, cx);
    fixture.hold("layout-preview:shell.terminal", at, cx);

    let marker = fixture.shown(MARKER, cx).expect("a line marks the place");
    let gap = fixture.bounds("layout-preview-gap:right:1", cx);
    assert!(
        inside(marker, gap),
        "{marker:?} in front of transfers, in {gap:?}"
    );

    fixture.release(at, cx);
    assert!(ended(&fixture, cx));
}

#[gpui_kit::test]
fn a_widget_released_over_a_panel_of_a_group_ends_the_drag_and_lands_beside_the_group(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    let files = fixture.bounds("layout-preview:shell.files", cx);
    fixture.hold("layout-preview:tools.clock", files.center(), cx);
    assert!(
        fixture.shown(GHOST, cx).is_some(),
        "the ghost follows the pointer"
    );

    fixture.release(files.center(), cx);

    assert!(ended(&fixture, cx), "the frame after the release is drawn");
    assert_eq!(
        fixture.left_to_right(&["group:navigation", "tools.clock", "group:assistant"], cx),
        ["group:navigation", "tools.clock", "group:assistant"]
    );
}

#[gpui_kit::test]
fn a_release_that_changes_nothing_ends_the_drag_and_leaves_no_ghost_or_mark(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    for target in [
        "layout-preview:shell.transfers",
        "layout-preview:tools.clock",
    ] {
        let at = across(&fixture, target, 0.25, cx);
        fixture.hold("layout-preview:shell.transfers", at, cx);
        assert!(
            fixture.shown(MARKER, cx).is_none(),
            "{target}: no place to mark"
        );

        fixture.release(at, cx);
        assert!(ended(&fixture, cx), "{target}");
    }
    assert_eq!(fixture.stored(cx), LayoutSettings::default());
}

#[gpui_kit::test]
fn a_release_away_from_every_chip_lands_in_the_nearest_zone(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let (preview, zone) = (
        fixture.bounds("layout-preview", cx),
        fixture.bounds("layout-preview-zone:right", cx),
    );
    let below_the_end = point(zone.right() - px(1.), preview.bottom() - px(1.));
    fixture.hold("layout-preview:shell.terminal", below_the_end, cx);
    fixture.release(below_the_end, cx);

    assert!(ended(&fixture, cx));
    assert_eq!(
        fixture.left_to_right(&["shell.terminal", "off.item"], cx),
        ["off.item", "shell.terminal"],
        "last in the right zone"
    );
}

#[gpui_kit::test]
fn a_panel_held_over_the_middle_of_a_panel_on_its_own_tints_that_panel(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose("layout-preview:shell.files", "Take out of group", cx);
    let files = fixture.bounds("layout-preview:shell.files", cx);
    fixture.hold("layout-preview:shell.agent", files.center(), cx);

    let tint = fixture.shown(MARKER, cx).expect("the panel is tinted");
    assert!(inside(tint, files), "{tint:?} on {files:?}");

    fixture.release(files.center(), cx);
    assert!(ended(&fixture, cx));
    assert!(
        fixture.has("layout-preview-group:group-1", cx),
        "the two make a group"
    );
}

#[gpui_kit::test]
fn the_ghost_of_a_drag_is_as_large_as_the_chip_or_the_group_it_lifts(cx: &mut TestAppContext) {
    let fixture = open(cx);
    for id in [
        "layout-preview:shell.transfers",
        "layout-preview-group:navigation",
    ] {
        let item = fixture.bounds(id, cx);
        let away = item.center() + point(px(0.), px(80.));
        fixture.hold(id, away, cx);
        let ghost = fixture.shown(GHOST, cx).expect("a ghost");
        let root = fixture.shown("drag-preview", cx).expect("the drag root");
        fixture.release(away, cx);

        assert_eq!(root.size, item.size, "{id}: the box it was measured in");
        assert_eq!(ghost.size, item.size, "{id}: the ghost fills it");
    }
}
