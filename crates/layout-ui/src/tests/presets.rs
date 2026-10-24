//! Choosing, duplicating, renaming and deleting layouts.

use gpui_kit::TestAppContext;
use nocterm_window_layout::{LayoutPreset, LayoutSettings, PresetId};

use super::{open, open_with};

fn pid(id: &str) -> PresetId {
    PresetId::from_owned(id.to_owned())
}

fn saved(id: &str, name: &str) -> LayoutPreset {
    LayoutPreset {
        id: pid(id),
        name: name.to_owned(),
        ..LayoutPreset::default()
    }
}

fn two_saved() -> LayoutSettings {
    LayoutSettings {
        active: Some(pid("p1")),
        presets: vec![saved("p1", "Work"), saved("p2", "Home")],
    }
}

#[gpui_kit::test]
fn showing_another_layout_does_not_switch_the_one_in_use(cx: &mut TestAppContext) {
    let fixture = open_with(two_saved(), cx);
    fixture.click("layout-choice:p2", cx);

    assert_eq!(fixture.stored(cx), two_saved());
}

#[gpui_kit::test]
fn a_built_in_layout_not_in_use_is_read_only_until_it_is_duplicated(cx: &mut TestAppContext) {
    let fixture = open_with(two_saved(), cx);
    fixture.click("layout-choice:swapped", cx);
    fixture.click("layout-item:shell.transfers", cx);

    assert!(fixture.has("layout-read-only", cx));
    assert!(
        !fixture.has("layout-op-earlier", cx),
        "no controls to change it"
    );

    fixture.click("layout-duplicate-to-edit", cx);

    let stored = fixture.stored(cx);
    assert_eq!(stored.presets.len(), 3);
    assert_eq!(
        stored.active,
        Some(pid("p1")),
        "the layout in use is the same"
    );
    assert!(
        !fixture.has("layout-read-only", cx),
        "the copy is shown and editable"
    );
}

#[gpui_kit::test]
fn duplicating_the_default_layout_leaves_it_in_use(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-duplicate", cx);

    let stored = fixture.stored(cx);
    assert_eq!(stored.presets.len(), 1);
    assert_eq!(stored.active, None);
}

#[gpui_kit::test]
fn a_layout_is_renamed_with_enter(cx: &mut TestAppContext) {
    let fixture = open_with(two_saved(), cx);
    fixture.click("layout-rename", cx);
    fixture.keys("ctrl-a", cx);
    fixture.keys("O f f i c e enter", cx);

    assert_eq!(fixture.stored(cx).presets[0].name, "Office");
    assert!(!fixture.has("layout-rename-row", cx));
}

#[gpui_kit::test]
fn a_name_that_is_taken_is_refused_beside_the_field(cx: &mut TestAppContext) {
    let fixture = open_with(two_saved(), cx);
    fixture.click("layout-rename", cx);
    fixture.keys("ctrl-a", cx);
    fixture.keys("H o m e", cx);
    fixture.click("layout-rename-save", cx);

    assert!(fixture.has("layout-rename-row", cx), "still asking");
    assert!(fixture.has("layout-rename-error", cx));
    assert_eq!(fixture.stored(cx).presets[0].name, "Work");

    fixture.click("layout-rename-cancel", cx);
    assert!(!fixture.has("layout-rename-row", cx));
}

#[gpui_kit::test]
fn a_blank_name_is_refused(cx: &mut TestAppContext) {
    let fixture = open_with(two_saved(), cx);
    fixture.click("layout-rename", cx);
    fixture.keys("ctrl-a backspace", cx);
    fixture.click("layout-rename-save", cx);

    assert!(fixture.has("layout-rename-error", cx));
    assert_eq!(fixture.stored(cx).presets[0].name, "Work");
}

#[gpui_kit::test]
fn deleting_a_layout_asks_first_and_can_be_undone(cx: &mut TestAppContext) {
    let fixture = open_with(two_saved(), cx);
    fixture.click("layout-delete", cx);
    assert!(fixture.has("layout-delete-confirm", cx));
    assert_eq!(
        fixture.stored(cx).presets.len(),
        2,
        "nothing is deleted yet"
    );

    fixture.click("layout-delete-cancel", cx);
    assert!(!fixture.has("layout-delete-confirm", cx));

    fixture.click("layout-delete", cx);
    fixture.click("layout-delete-confirmed", cx);
    assert_eq!(fixture.stored(cx).presets.len(), 1);
    assert_eq!(
        fixture.label("layout-notice", cx).as_deref(),
        Some("Deleted \u{201c}Work\u{201d}."),
        "the notice names the layout that went"
    );

    fixture.click("layout-notice-undo", cx);
    assert_eq!(fixture.stored(cx).presets, two_saved().presets);
}

#[gpui_kit::test]
fn resetting_a_layout_arranges_it_by_the_hints_again_and_can_be_undone(cx: &mut TestAppContext) {
    let fixture = open_with(two_saved(), cx);
    fixture.click("layout-item:shell.transfers", cx);
    fixture.click("layout-op-earlier", cx);
    let moved = fixture.stored(cx);
    assert_ne!(moved.presets[0].bar, LayoutPreset::default().bar);

    fixture.click("layout-reset", cx);
    assert_eq!(
        fixture.stored(cx).presets[0].bar,
        LayoutPreset::default().bar
    );
    assert_eq!(
        fixture.stored(cx).presets[0].name,
        "Work",
        "only the arrangement"
    );
    assert_eq!(
        fixture.label("layout-notice", cx).as_deref(),
        Some("Reset \u{201c}Work\u{201d}.")
    );

    fixture.click("layout-notice-undo", cx);
    assert_eq!(fixture.stored(cx), moved);
}

#[gpui_kit::test]
fn a_built_in_layout_cannot_be_renamed_or_deleted(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-rename", cx);
    fixture.click("layout-delete", cx);
    fixture.click("layout-reset", cx);

    assert!(fixture.stored(cx).presets.is_empty(), "nothing was written");
    assert!(!fixture.has("layout-rename-row", cx));
    assert!(!fixture.has("layout-delete-confirm", cx));
}
