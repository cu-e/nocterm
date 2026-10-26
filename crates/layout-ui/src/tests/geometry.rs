//! The editor lays itself out inside the width it is given: the bar stays one line inside its
//! box and the tray stays below it, at a Settings page's width and at a narrow one.

use gpui_kit::{Bounds, Pixels, TestAppContext, test::TestWindowExt as _};

use super::{Fixture, open};

/// The widths tried: a wide window, a Settings page beside its navigation, a narrow window.
const WIDTHS: [f32; 3] = [1200., 660., 420.];

/// The widths at which the whole bar of the default layout fits, in full, packed or as icons.
const FITTING: [f32; 3] = [1200., 660., 580.];

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
fn the_tray_stays_below_the_bar_and_inside_the_editor(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.drag("layout-preview:tools.clock", "layout-tray", cx);
    for width in WIDTHS {
        fixture.resize(width, cx);
        let tray = bounds(&fixture, "layout-tray", cx);
        let preview = bounds(&fixture, "layout-preview", cx);
        let chip = bounds(&fixture, "layout-tray:tools.clock", cx);
        assert!(!overlap(tray, preview), "{width}: {tray:?} / {preview:?}");
        assert!(preview.bottom() <= tray.top(), "{width}: below");
        assert!(inside(chip, tray), "{width}: {chip:?} in {tray:?}");
        assert!(inside(tray, bounds(&fixture, "layout-editor", cx)));
    }
}
