//! Moving, grouping and removing items edits the layout in use.

use gpui_kit::TestAppContext;
use nocterm_settings::SettingsDocument;
use nocterm_window_layout::{LayoutSettings, PresetId};

use super::{RIGHT, open, open_over, open_with};

#[gpui_kit::test]
fn the_first_move_in_the_default_layout_saves_an_active_custom_layout(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.transfers", cx);
    fixture.keys("alt-left", cx);

    let stored = fixture.stored(cx);
    assert_eq!(stored.presets.len(), 1);
    assert_eq!(stored.presets[0].name, "Custom");
    assert_eq!(stored.active, Some(PresetId::from_owned("p1".to_owned())));
    assert_eq!(
        fixture.left_to_right(&RIGHT, cx),
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
    fixture.click("layout-preview:shell.transfers", cx);
    fixture.keys("alt-right alt-right", cx);

    let stored = fixture.stored(cx);
    assert_eq!(stored.presets.len(), 1, "one Custom layout, not two");
    assert_eq!(stored.active, Some(PresetId::from_owned("p1".to_owned())));
    assert_eq!(
        fixture.left_to_right(&RIGHT, cx),
        [
            "shell.notices",
            "tools.clock",
            "shell.transfers",
            "shell.terminal"
        ]
    );
}

#[gpui_kit::test]
fn an_item_moves_to_another_zone_from_its_menu(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose("layout-preview:shell.transfers", "Move to Left", cx);

    assert_eq!(
        fixture.left_to_right(&["shell.files", "shell.transfers", "shell.notices"], cx),
        ["shell.files", "shell.transfers", "shell.notices"],
    );
}

#[gpui_kit::test]
fn a_pinned_item_cannot_be_removed_from_the_bar(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.notices", cx);
    fixture.keys("delete", cx);

    assert!(fixture.stored(cx).presets.is_empty(), "nothing was written");
    assert!(fixture.has("layout-preview:shell.notices", cx));
}

#[gpui_kit::test]
fn a_hidden_item_waits_in_the_tray_until_it_is_added_back(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose("layout-preview:tools.clock", "Hide from the bar", cx);

    assert!(!fixture.has("layout-preview:tools.clock", cx));
    assert!(fixture.has("layout-tray:tools.clock", cx));

    fixture.click("layout-tray:tools.clock", cx);
    assert!(fixture.has("layout-preview:tools.clock", cx));
    assert!(!fixture.has("layout-tray:tools.clock", cx));
}

#[gpui_kit::test]
fn the_delete_key_removes_the_selected_item(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.terminal", cx);
    fixture.keys("delete", cx);

    assert!(fixture.has("layout-tray:shell.terminal", cx));
    assert!(!fixture.has("layout-preview:shell.terminal", cx));
}

#[gpui_kit::test]
fn a_click_selects_the_chip_the_keys_act_on(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.click("layout-preview:shell.terminal", cx);

    assert!(fixture.is_selected("shell.terminal", cx));
    assert!(!fixture.is_selected("shell.transfers", cx));

    fixture.click("layout-preview-group:navigation", cx);
    assert!(fixture.is_selected("group:navigation", cx));
    assert!(!fixture.is_selected("shell.terminal", cx));
}

#[gpui_kit::test]
fn a_separator_is_added_after_the_item_and_removed_from_its_own_menu(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose("layout-preview:shell.transfers", "Add separator after", cx);

    let separator = "layout-preview-separator:right:2";
    assert!(fixture.has(separator, cx));
    fixture.choose(separator, "Remove separator", cx);
    assert!(!fixture.has(separator, cx));
}

#[gpui_kit::test]
fn a_panel_opens_with_the_layout_when_its_menu_says_so(cx: &mut TestAppContext) {
    let fixture = open(cx);
    fixture.choose("layout-preview:shell.files", "Open with the layout", cx);

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
fn an_item_of_a_disabled_plugin_is_marked_and_still_shown(cx: &mut TestAppContext) {
    let fixture = open(cx);

    assert_eq!(
        fixture.label("layout-preview:off.item", cx).as_deref(),
        Some("Dormant (disabled)")
    );
    assert_eq!(
        fixture.label("layout-preview:tools.clock", cx).as_deref(),
        Some("Clock")
    );
}

#[gpui_kit::test]
fn a_layouts_table_that_does_not_read_is_not_overwritten(cx: &mut TestAppContext) {
    let table = "[layouts]\nactive = \"p1\"\n[[layouts.presets]]\nid = \"p1\"\n\
                 [layouts.presets.bar]\nleft = [{ separator = false }]\n";
    let mut document = SettingsDocument::from_table(table.parse().expect("the text is toml"));
    document.register::<LayoutSettings>();
    let written = document.to_table().get("layouts").cloned();
    let fixture = open_over(document, cx);

    fixture.click("layout-preview:shell.transfers", cx);
    fixture.keys("alt-left", cx);

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
