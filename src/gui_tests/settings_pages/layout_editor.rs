//! The status bar and panels editor inside Appearance, in the composed application.
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, test::TestWindowExt as _,
};
use nocterm_settings::SettingsDocument;
use nocterm_settings_ui::SettingsView;
use nocterm_store::{SettingsConfig, SettingsHandle};
use nocterm_window_layout::{BuiltIn, LayoutSettings, select};
use nocterm_workspace::{OpenLayoutSettings, OpenPlugins, Workspace};

fn window(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    let (handle, workspace, _) = window_with_settings(cx);
    (handle, workspace)
}

fn window_with_settings(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>, SettingsHandle) {
    window_of_height(3000., cx)
}

/// A window `height` pixels tall; the editor is whole on screen only when that is generous.
fn window_of_height(
    height: f32,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>, SettingsHandle) {
    let config = SettingsConfig::in_memory(SettingsDocument::default());
    let (handle, workspace, _, _, _, settings) =
        super::super::fixture_with_settings(cx, false, None, false, Some(config));
    cx.simulate_window_resize(
        handle,
        gpui_kit::size(gpui_kit::px(1600.), gpui_kit::px(height)),
    );
    cx.run_until_parked();
    (handle, workspace, settings)
}

fn click(handle: AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id.to_owned(), cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn saved_layouts(settings: &SettingsHandle, cx: &mut TestAppContext) -> usize {
    cx.update(|cx| settings.setting::<LayoutSettings>(cx).presets.len())
}

/// The summary of the host's resources in the footer, which the default layout places.
const FOOTER_MONITOR: &str = "host-monitor";

fn dispatch(handle: AnyWindowHandle, action: impl gpui_kit::Action, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(action), cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn selected_page(workspace: &Entity<Workspace>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| {
        let view = workspace
            .read(cx)
            .find_item::<SettingsView>(cx)
            .expect("the Settings tab is open");
        view.read(cx).selected_page_id().to_owned()
    })
}

fn shown(handle: AnyWindowHandle, id: &str, cx: &mut TestAppContext) -> bool {
    cx.update_window(handle, |_, window, _| {
        window.try_find(id.to_owned()).is_some()
    })
    .unwrap()
}

fn first_contribution_of(plugin: &str) -> String {
    let catalog = crate::shell_setup::plugin_catalog().expect("the application catalog");
    let plugin = catalog
        .plugins()
        .find(|candidate| candidate.id.as_str() == plugin)
        .expect("the plugin is in the catalog");
    plugin.contributions[0].id.to_string()
}

#[gpui_kit::test]
fn the_editor_is_a_section_of_appearance(cx: &mut TestAppContext) {
    let (handle, workspace) = window(cx);
    dispatch(handle, OpenLayoutSettings { select: None }, cx);
    assert_eq!(selected_page(&workspace, cx), "appearance");
    assert!(shown(handle, "settings-section:layout", cx));
    assert!(shown(handle, "layout-editor", cx));
}

#[gpui_kit::test]
fn the_action_selects_the_item_it_names(cx: &mut TestAppContext) {
    let (handle, _) = window(cx);
    let id = first_contribution_of("monitor");
    dispatch(
        handle,
        OpenLayoutSettings {
            select: Some(id.clone().into()),
        },
        cx,
    );
    assert!(shown(handle, &format!("layout-inspector:{id}"), cx));
}

#[gpui_kit::test]
fn the_action_scrolls_the_editor_into_view_of_a_short_window(cx: &mut TestAppContext) {
    let (handle, _, _) = window_of_height(500., cx);
    let id = first_contribution_of("monitor");
    dispatch(
        handle,
        OpenLayoutSettings {
            select: Some(id.into()),
        },
        cx,
    );
    // The scroll offset is applied one frame after the page asks for it.
    for _ in 0..3 {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        })
        .unwrap();
    }

    let (view, section) = cx
        .update_window(handle, |_, window, _| {
            (
                window.find("settings-view").bounds(),
                window.find("settings-section:layout").bounds(),
            )
        })
        .unwrap();
    assert!(
        section.origin.y >= view.origin.y && section.origin.y < view.origin.y + view.size.height,
        "the editor starts inside the page: section {section:?}, page {view:?}"
    );
}

#[gpui_kit::test]
fn a_plugin_links_from_the_plugins_page_to_its_item_in_the_editor(cx: &mut TestAppContext) {
    let (handle, workspace) = window(cx);
    dispatch(
        handle,
        OpenPlugins {
            plugin: Some("monitor".into()),
        },
        cx,
    );
    cx.update_window(handle, |_, window, cx| {
        window.click("plugin-layout-link", cx)
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    assert_eq!(selected_page(&workspace, cx), "appearance");
    let id = first_contribution_of("monitor");
    assert!(shown(handle, &format!("layout-inspector:{id}"), cx));
}

#[gpui_kit::test]
fn the_first_edit_of_the_default_layout_changes_the_footer_of_the_window(cx: &mut TestAppContext) {
    let (handle, _, settings) = window_with_settings(cx);
    let id = first_contribution_of("monitor");
    dispatch(
        handle,
        OpenLayoutSettings {
            select: Some(id.clone().into()),
        },
        cx,
    );
    assert!(shown(handle, FOOTER_MONITOR, cx), "the default footer");

    click(handle, "layout-op-remove", cx);

    assert_eq!(saved_layouts(&settings, cx), 1);
    assert!(
        !shown(handle, FOOTER_MONITOR, cx),
        "the footer follows the edit at once"
    );
}

#[gpui_kit::test]
fn duplicating_a_layout_after_an_undo_leaves_the_layout_in_use_alone(cx: &mut TestAppContext) {
    let (handle, _, settings) = window_with_settings(cx);
    let id = first_contribution_of("monitor");
    dispatch(
        handle,
        OpenLayoutSettings {
            select: Some(id.into()),
        },
        cx,
    );
    click(handle, "layout-op-remove", cx);
    click(handle, "layout-undo", cx);
    assert!(
        shown(handle, FOOTER_MONITOR, cx),
        "the undo brought it back"
    );

    // The layout in use is Default again, and the first edit left its old id behind.
    click(handle, "layout-choice:swapped", cx);
    click(handle, "layout-duplicate", cx);

    let in_use = cx.update(|cx| select(settings.setting::<LayoutSettings>(cx), false));
    assert_eq!(in_use, BuiltIn::Default.id(), "the copy did not take over");
    assert!(
        shown(handle, FOOTER_MONITOR, cx),
        "the window keeps the layout in use"
    );
    assert_eq!(saved_layouts(&settings, cx), 1, "the copy is saved");
}

#[gpui_kit::test]
fn the_monitor_form_writes_its_own_setting_and_not_a_layout(cx: &mut TestAppContext) {
    let (handle, _, settings) = window_with_settings(cx);
    let id = first_contribution_of("monitor");
    dispatch(
        handle,
        OpenLayoutSettings {
            select: Some(id.into()),
        },
        cx,
    );
    let before = cx.update(|cx| {
        settings
            .setting::<nocterm_monitor::MonitorSettings>(cx)
            .status_bar
            .clone()
    });

    click(handle, "plugin-setting:monitor:status_bar:swap", cx);

    let after = cx.update(|cx| {
        settings
            .setting::<nocterm_monitor::MonitorSettings>(cx)
            .status_bar
            .clone()
    });
    assert_ne!(
        before, after,
        "the metric was added to [monitor].status_bar"
    );
    assert_eq!(saved_layouts(&settings, cx), 0, "no layout was written");
}
