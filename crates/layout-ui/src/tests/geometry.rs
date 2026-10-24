//! The editor lays itself out inside the width it is given: the bar stays inside its box, the
//! zone columns, the tray and the inspector never overlap, at a Settings page's width and at a
//! narrow one.

use gpui_kit::{Bounds, Pixels, TestAppContext, test::TestWindowExt as _};

use super::{Fixture, open};

/// The widths tried: a wide window, a Settings page beside its navigation, a narrow window.
const WIDTHS: [f32; 3] = [1200., 660., 420.];

/// The widths at which the whole bar of the default layout fits, in full, packed or as icons.
const FITTING: [f32; 3] = [1200., 660., 580.];

/// The rows of the lists that the default layout places.
const ROWS: [&str; 9] = [
    "layout-item:group:navigation",
    "layout-item:shell.servers",
    "layout-item:shell.files",
    "layout-item:shell.notices",
    "layout-item:shell.transfers",
    "layout-item:tools.clock",
    "layout-item:shell.terminal",
    "layout-item:plain.note",
    "layout-item:off.item",
];

fn overlap(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    let slack = gpui_kit::px(0.5);
    a.left() + slack < b.right()
        && b.left() + slack < a.right()
        && a.top() + slack < b.bottom()
        && b.top() + slack < a.bottom()
}

fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
    let slack = gpui_kit::px(0.5);
    inner.left() + slack >= outer.left()
        && inner.right() <= outer.right() + slack
        && inner.top() + slack >= outer.top()
        && inner.bottom() <= outer.bottom() + slack
}

fn bounds(fixture: &Fixture, id: &str, cx: &mut TestAppContext) -> Bounds<Pixels> {
    let mut found = None;
    fixture.frame(cx, |window, _| {
        found = window
            .try_find(id.to_owned())
            .map(|element| element.bounds());
    });
    found.unwrap_or_else(|| panic!("{id} is not in the frame"))
}

/// The chips of the bar preview that the default layout places.
const CHIPS: [&str; 10] = [
    "layout-preview:shell.servers",
    "layout-preview:shell.files",
    "layout-preview:shell.agent",
    "layout-preview:shell.notices",
    "layout-preview:shell.transfers",
    "layout-preview:tools.clock",
    "layout-preview:tools.cpu",
    "layout-preview:shell.terminal",
    "layout-preview:plain.note",
    "layout-preview:off.item",
];

fn centre_y(bounds: Bounds<Pixels>) -> f32 {
    f32::from(bounds.origin.y + bounds.size.height / 2.)
}

/// The chips of the preview in the frame, with their bounds.
fn chips(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<(&'static str, Bounds<Pixels>)> {
    let mut found = Vec::new();
    for id in CHIPS {
        if fixture.has(id, cx) {
            found.push((id, bounds(fixture, id, cx)));
        }
    }
    found
}

#[gpui_kit::test]
fn the_bar_preview_is_one_line_inside_its_box_at_every_width(cx: &mut TestAppContext) {
    let fixture = open(cx);
    for width in FITTING {
        fixture.resize(width, cx);
        let editor = bounds(&fixture, "layout-editor", cx);
        let preview = bounds(&fixture, "layout-preview", cx);
        assert!(
            inside(preview, editor),
            "{width}: {preview:?} in {editor:?}"
        );
        let chips = chips(&fixture, cx);
        assert!(chips.len() >= 6, "{width}: the bar was drawn");
        let line = centre_y(chips[0].1);
        for (id, chip) in &chips {
            assert!(
                (centre_y(*chip) - line).abs() < 0.5,
                "{width}: {id} {chip:?} is not on the line at {line}"
            );
            assert!(
                inside(*chip, preview),
                "{width}: {id} {chip:?} in {preview:?}"
            );
        }
        for zone in ["left", "center", "right"] {
            let zone = bounds(&fixture, &format!("layout-preview-zone:{zone}"), cx);
            assert!(inside(zone, preview), "{width}: {zone:?} in {preview:?}");
            assert!((centre_y(zone) - line).abs() < 0.5, "{width}: {zone:?}");
        }
    }
}

#[gpui_kit::test]
fn the_bar_preview_names_its_chips_when_there_is_room_and_only_shows_icons_when_not(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    let id = "layout-preview:shell.transfers";

    fixture.resize(2800., cx);
    let roomy = bounds(&fixture, id, cx);
    fixture.resize(660., cx);
    let tight = bounds(&fixture, id, cx);

    assert!(
        roomy.size.width > tight.size.width + gpui_kit::px(20.),
        "{roomy:?} / {tight:?}"
    );
    assert!(fixture.has("layout-preview-line", cx), "one line in both");
}

#[gpui_kit::test]
fn the_bar_preview_keeps_the_zones_in_order_from_left_to_right(cx: &mut TestAppContext) {
    let fixture = open(cx);
    for width in FITTING {
        fixture.resize(width, cx);
        let left = bounds(&fixture, "layout-preview-zone:left", cx);
        let centre = bounds(&fixture, "layout-preview-zone:center", cx);
        let right = bounds(&fixture, "layout-preview-zone:right", cx);
        assert!(left.right() <= centre.left() + gpui_kit::px(0.5), "{width}");
        assert!(
            centre.right() <= right.left() + gpui_kit::px(0.5),
            "{width}"
        );
    }
}

#[gpui_kit::test]
fn the_bar_preview_in_a_very_narrow_box_scrolls_instead_of_wrapping(cx: &mut TestAppContext) {
    let fixture = open(cx);
    for width in [420., 180.] {
        fixture.resize(width, cx);
        let editor = bounds(&fixture, "layout-editor", cx);
        let preview = bounds(&fixture, "layout-preview", cx);
        assert!(
            inside(preview, editor),
            "{width}: {preview:?} in {editor:?}"
        );
        let chips = chips(&fixture, cx);
        let line = centre_y(chips[0].1);
        assert!(
            chips
                .iter()
                .all(|(_, chip)| (centre_y(*chip) - line).abs() < 0.5),
            "{width}: one line: {chips:?}"
        );
    }
}

#[gpui_kit::test]
fn the_inspector_overlaps_neither_the_bar_nor_the_lists(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:tools.clock", cx);
    for width in WIDTHS {
        fixture.resize(width, cx);
        let preview = bounds(&fixture, "layout-preview", cx);
        let lists = bounds(&fixture, "layout-lists-column", cx);
        let inspector = bounds(&fixture, "layout-inspector", cx);
        let editor = bounds(&fixture, "layout-editor", cx);
        assert!(
            !overlap(inspector, preview),
            "{width}: {inspector:?} / {preview:?}"
        );
        assert!(
            !overlap(inspector, lists),
            "{width}: {inspector:?} / {lists:?}"
        );
        assert!(!overlap(lists, preview), "{width}: {lists:?} / {preview:?}");
        assert!(
            inside(inspector, editor),
            "{width}: {inspector:?} in {editor:?}"
        );
        assert!(inside(lists, editor), "{width}: {lists:?} in {editor:?}");
    }
}

#[gpui_kit::test]
fn the_inspector_sits_beside_the_lists_when_there_is_room_and_below_them_when_not(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    fixture.click("layout-item:tools.clock", cx);

    fixture.resize(1200., cx);
    let (lists, inspector) = (
        bounds(&fixture, "layout-lists-column", cx),
        bounds(&fixture, "layout-inspector", cx),
    );
    assert!(
        lists.right() <= inspector.left(),
        "{lists:?} / {inspector:?}"
    );

    fixture.resize(420., cx);
    let (lists, inspector) = (
        bounds(&fixture, "layout-lists-column", cx),
        bounds(&fixture, "layout-inspector", cx),
    );
    assert!(
        lists.bottom() <= inspector.top(),
        "{lists:?} / {inspector:?}"
    );
}

#[gpui_kit::test]
fn the_zone_columns_do_not_overlap_and_keep_their_rows_inside(cx: &mut TestAppContext) {
    let fixture = open(cx);
    for width in WIDTHS {
        fixture.resize(width, cx);
        let columns = ["left", "center", "right"].map(|name| {
            let id = format!("layout-zone:{name}");
            (id.clone(), bounds(&fixture, &id, cx))
        });
        for (at, (name, one)) in columns.iter().enumerate() {
            for (other, two) in columns.iter().skip(at + 1) {
                assert!(
                    !overlap(*one, *two),
                    "{width}: {name} {one:?} / {other} {two:?}"
                );
            }
        }
        let lists = bounds(&fixture, "layout-lists-column", cx);
        let mut rows = 0;
        for id in ROWS {
            if !fixture.has(id, cx) {
                continue;
            }
            rows += 1;
            let row = bounds(&fixture, id, cx);
            let column = columns.iter().find(|(_, column)| inside(row, *column));
            assert!(
                column.is_some(),
                "{width}: {id} {row:?} is in no column {columns:?}"
            );
            assert!(inside(row, lists), "{width}: {id} {row:?} in {lists:?}");
        }
        assert!(rows >= 8, "{width}: the rows were drawn");
    }
}

#[gpui_kit::test]
fn the_tray_stays_below_the_zone_columns_and_inside_the_editor(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:tools.clock", cx);
    fixture.click("layout-op-remove", cx);
    for width in WIDTHS {
        fixture.resize(width, cx);
        let tray = bounds(&fixture, "layout-tray", cx);
        let zones = bounds(&fixture, "layout-zones", cx);
        let chip = bounds(&fixture, "layout-tray:tools.clock", cx);
        assert!(!overlap(tray, zones), "{width}: {tray:?} / {zones:?}");
        assert!(inside(chip, tray), "{width}: {chip:?} in {tray:?}");
        assert!(inside(tray, bounds(&fixture, "layout-editor", cx)));
    }
}
