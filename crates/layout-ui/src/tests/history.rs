//! Undo and redo restore the layouts the editor changed, and give way to outside changes.

use gpui_kit::TestAppContext;
use nocterm_window_layout::LayoutSettings;

use super::{RIGHT, open, open_on_disk};

const ORIGINAL: [&str; 4] = [
    "shell.notices",
    "shell.transfers",
    "tools.clock",
    "shell.terminal",
];

#[gpui_kit::test]
fn undo_puts_an_item_back_and_redo_moves_it_again(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.transfers", cx);
    fixture.keys("alt-left", cx);
    let moved = fixture.left_to_right(&RIGHT, cx);
    assert_ne!(moved, ORIGINAL);

    fixture.click("layout-undo", cx);
    assert_eq!(fixture.left_to_right(&RIGHT, cx), ORIGINAL);

    fixture.click("layout-redo", cx);
    assert_eq!(fixture.left_to_right(&RIGHT, cx), moved);
}

#[gpui_kit::test]
fn the_keyboard_undoes_and_redoes(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.transfers", cx);
    fixture.keys("alt-right", cx);
    let moved = fixture.left_to_right(&RIGHT, cx);

    fixture.keys("ctrl-z", cx);
    assert_eq!(fixture.left_to_right(&RIGHT, cx), ORIGINAL);

    fixture.keys("ctrl-shift-z", cx);
    assert_eq!(fixture.left_to_right(&RIGHT, cx), moved);
}

#[gpui_kit::test]
fn there_is_nothing_to_undo_before_an_edit(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-undo", cx);
    fixture.keys("ctrl-z", cx);

    assert_eq!(fixture.stored(cx), LayoutSettings::default());
}

#[gpui_kit::test]
fn a_change_from_elsewhere_ends_the_history(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.transfers", cx);
    fixture.keys("alt-left", cx);

    // Another window renames the layout the editor just saved.
    cx.update(|cx| {
        drop(fixture.settings.update::<LayoutSettings>(cx, |layouts| {
            layouts.presets[0].name = "Elsewhere".to_owned();
        }));
    });
    cx.run_until_parked();
    fixture.click("layout-undo", cx);

    let stored = fixture.stored(cx);
    assert_eq!(
        stored.presets.len(),
        1,
        "undo would have overwritten the other change"
    );
    assert_eq!(stored.presets[0].name, "Elsewhere");
}

#[gpui_kit::test]
fn two_undos_pressed_before_the_first_is_stored_take_back_one_step_and_lose_none(
    cx: &mut TestAppContext,
) {
    let directory = tempfile::tempdir().unwrap();
    let fixture = open_on_disk(directory.path(), cx);
    fixture.click("layout-preview:shell.transfers", cx);
    fixture.keys("alt-left", cx);
    let first = fixture.wait_for(cx, |stored| stored.presets.len() == 1);
    fixture.click("layout-preview:shell.terminal", cx);
    fixture.keys("alt-left", cx);
    let second = fixture.wait_for(cx, |stored| stored != &first);

    // Both presses arrive while the file is still being written.
    fixture.keys("ctrl-z ctrl-z", cx);
    let undone = fixture.wait_for(cx, |stored| stored != &second);
    assert_eq!(undone, first, "one step back, not two");

    fixture.keys("ctrl-z", cx);
    let start = fixture.wait_for(cx, |stored| stored.presets.is_empty());
    assert!(
        start.presets.is_empty(),
        "the first edit could still be undone"
    );
    let mut on_disk = nocterm_settings::SettingsFile::new(directory.path().join("settings.toml"))
        .load()
        .unwrap();
    on_disk.register::<LayoutSettings>();
    assert_eq!(on_disk.get::<LayoutSettings>().presets, start.presets);
}

#[gpui_kit::test]
fn an_undo_pressed_right_after_a_move_leaves_the_move_alone(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.terminal", cx);
    fixture.keys("alt-left", cx);
    let first = fixture.left_to_right(&RIGHT, cx);
    fixture.click("layout-preview:shell.transfers", cx);

    // The undo would restore the layouts from before the first move, and lose the second.
    fixture.keys("alt-right ctrl-z", cx);
    let second = fixture.left_to_right(&RIGHT, cx);
    assert_ne!(second, first);
    assert_ne!(second, ORIGINAL, "the second move is still there");

    fixture.keys("ctrl-z", cx);
    assert_eq!(fixture.left_to_right(&RIGHT, cx), first);
    fixture.keys("ctrl-z", cx);
    assert_eq!(fixture.left_to_right(&RIGHT, cx), ORIGINAL);
}
