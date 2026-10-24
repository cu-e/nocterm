//! Moving, grouping and removing items edits the layout in use.

use gpui_kit::{TestAppContext, test::TestWindowExt as _};
use nocterm_settings::SettingsDocument;
use nocterm_window_layout::{LayoutSettings, PresetId};

use super::{RIGHT, open, open_over, open_with};

#[gpui_kit::test]
fn the_first_move_in_the_default_layout_saves_an_active_custom_layout(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:shell.transfers", cx);
    fixture.click("layout-op-earlier", cx);

    let stored = fixture.stored(cx);
    assert_eq!(stored.presets.len(), 1);
    assert_eq!(stored.presets[0].name, "Custom");
    assert_eq!(stored.active, Some(PresetId::from_owned("p1".to_owned())));
    assert_eq!(
        fixture.top_to_bottom(&RIGHT, cx),
        [
            "shell.transfers",
            "shell.notices",
            "tools.clock",
            "shell.terminal"
        ]
    );
}

#[gpui_kit::test]
fn two_quick_moves_from_the_keyboard_land_in_one_custom_layout(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:shell.transfers", cx);
    fixture.keys("alt-right alt-right", cx);

    let stored = fixture.stored(cx);
    assert_eq!(stored.presets.len(), 1, "one Custom layout, not two");
    assert_eq!(stored.active, Some(PresetId::from_owned("p1".to_owned())));
    assert_eq!(
        fixture.top_to_bottom(&RIGHT, cx),
        [
            "shell.notices",
            "tools.clock",
            "shell.transfers",
            "shell.terminal"
        ]
    );
}

#[gpui_kit::test]
fn an_item_moves_to_another_zone_from_the_inspector(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:shell.transfers", cx);
    fixture.click("layout-op-zone-left", cx);

    let mut columns = Vec::new();
    fixture.frame(cx, |window, _| {
        for id in ["layout-item:shell.transfers", "layout-item:shell.notices"] {
            columns.push(f32::from(window.find(id).bounds().origin.x));
        }
    });
    assert!(
        columns[0] < columns[1],
        "transfers left of notices: {columns:?}"
    );
}

#[gpui_kit::test]
fn a_pinned_item_cannot_be_removed_from_the_bar(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:shell.notices", cx);
    fixture.click("layout-op-remove", cx);
    fixture.keys("delete", cx);

    assert!(fixture.stored(cx).presets.is_empty(), "nothing was written");
    assert!(fixture.has("layout-pinned:shell.notices", cx));
    assert!(fixture.has("layout-item:shell.notices", cx));
}

#[gpui_kit::test]
fn a_removed_item_waits_in_the_tray_until_it_is_added_back(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:tools.clock", cx);
    fixture.click("layout-op-remove", cx);

    assert!(!fixture.has("layout-item:tools.clock", cx));
    assert!(fixture.has("layout-tray:tools.clock", cx));

    fixture.click("layout-tray-add:tools.clock", cx);
    assert!(fixture.has("layout-item:tools.clock", cx));
    assert!(!fixture.has("layout-tray:tools.clock", cx));
}

#[gpui_kit::test]
fn the_delete_key_removes_the_selected_item(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:shell.terminal", cx);
    fixture.keys("delete", cx);

    assert!(fixture.has("layout-tray:shell.terminal", cx));
    assert!(!fixture.has("layout-item:shell.terminal", cx));
}

#[gpui_kit::test]
fn a_separator_is_added_after_the_item_and_removed_again(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:shell.transfers", cx);
    fixture.click("layout-op-separator", cx);

    let separator = "layout-separator-remove:right:2";
    assert!(fixture.has(separator, cx));
    fixture.click(separator, cx);
    assert!(!fixture.has(separator, cx));
}

#[gpui_kit::test]
fn a_panel_opens_with_the_layout_when_its_switch_is_on(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-item:shell.files", cx);
    fixture.click("layout-open-by-default", cx);

    let stored = fixture.stored(cx);
    assert_eq!(stored.presets.len(), 1);
    assert_eq!(stored.presets[0].open.len(), 1);
    assert_eq!(stored.presets[0].open[0].as_str(), "shell.files");
}

#[gpui_kit::test]
fn a_stored_id_the_application_does_not_know_can_be_forgotten(cx: &mut TestAppContext) {
    let mut preset = nocterm_window_layout::LayoutPreset {
        id: PresetId::from_owned("p1".to_owned()),
        name: "Mine".to_owned(),
        ..Default::default()
    };
    preset
        .tray
        .push(nocterm_plugin_model::ContributionId::new("ghost.thing"));
    let stored = nocterm_window_layout::LayoutSettings {
        active: Some(preset.id.clone()),
        presets: vec![preset],
    };
    let fixture = open_with(stored, cx);

    assert!(fixture.has("layout-unknown:ghost.thing", cx));
    fixture.click("layout-forget:ghost.thing", cx);
    assert!(!fixture.has("layout-unknown:ghost.thing", cx));
}

#[gpui_kit::test]
fn an_item_of_a_disabled_plugin_is_marked_and_still_listed(cx: &mut TestAppContext) {
    let fixture = open(cx);

    assert!(fixture.has("layout-item:off.item", cx));
    assert!(fixture.has("layout-disabled:off.item", cx));
    assert!(!fixture.has("layout-disabled:tools.clock", cx));
}

#[gpui_kit::test]
fn a_layouts_table_that_does_not_read_is_not_overwritten(cx: &mut TestAppContext) {
    let table = "[layouts]\nactive = \"p1\"\n[[layouts.presets]]\nid = \"p1\"\n\
                 [layouts.presets.bar]\nleft = [{ separator = false }]\n";
    let mut document = SettingsDocument::from_table(table.parse().expect("the text is toml"));
    document.register::<LayoutSettings>();
    let written = document.to_table().get("layouts").cloned();
    let fixture = open_over(document, cx);

    fixture.click("layout-item:shell.transfers", cx);
    fixture.click("layout-op-earlier", cx);

    cx.update(|cx| {
        assert_eq!(
            fixture
                .settings
                .document(cx)
                .to_table()
                .get("layouts")
                .cloned(),
            written
        );
        assert_eq!(fixture.settings.revision(cx), 0, "nothing was written");
    });
    assert!(fixture.has("layout-notice", cx), "the user is told why");
}
