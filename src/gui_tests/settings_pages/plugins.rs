//! The Plugins page and the plugin pages nested under it, in the composed application.
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, test::TestWindowExt as _,
};
use nocterm_settings::SettingsDocument;
use nocterm_settings_ui::SettingsView;
use nocterm_store::{SettingsConfig, SettingsHandle};
use nocterm_workspace::{OpenPluginPage, OpenPlugins, Workspace};

fn window_over(
    toml: &str,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>, SettingsHandle) {
    let document = SettingsDocument::from_table(toml.parse().expect("the settings are toml"));
    let (handle, workspace, _, _, _, settings) = super::super::fixture_with_settings(
        cx,
        false,
        None,
        false,
        Some(SettingsConfig::in_memory(document)),
    );
    (handle, workspace, settings)
}

fn dispatch(handle: AnyWindowHandle, action: impl gpui_kit::Action, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(action), cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn settings_view(workspace: &Entity<Workspace>, cx: &mut TestAppContext) -> Entity<SettingsView> {
    cx.update(|cx| workspace.read(cx).find_item::<SettingsView>(cx))
        .expect("the Settings tab is open")
}

fn selected_page(workspace: &Entity<Workspace>, cx: &mut TestAppContext) -> String {
    let view = settings_view(workspace, cx);
    cx.update(|cx| view.read(cx).selected_page_id().to_owned())
}

fn render(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

fn detail_label(handle: AnyWindowHandle, cx: &mut TestAppContext) -> Option<String> {
    cx.update_window(handle, |_, window, _| {
        window
            .try_find("plugin-detail")
            .and_then(|detail| detail.label().map(str::to_owned))
    })
    .unwrap()
}

#[gpui_kit::test]
fn plugins_follows_the_other_guests_and_nested_pages_stay_out_of_the_list(cx: &mut TestAppContext) {
    let (handle, view, _) = super::open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let nav = |window: &mut gpui_kit::Window, index: usize| {
            window.within("settings-pages").try_find(index).is_some()
        };
        let listed: Vec<_> = (0..12).filter(|index| nav(window, *index)).collect();
        assert_eq!(
            listed,
            [0, 1, 2, 3, 4, 5, 7, 9],
            "Monitor (6) and Workspaces (8) are nested under Plugins"
        );
        window.within("settings-pages").click(7usize, cx);
        assert_eq!(view.read(cx).selected_page_id(), "explorer");
        window.within("settings-pages").click(9usize, cx);
        assert_eq!(view.read(cx).selected_page_id(), "plugins");
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_nested_page_belongs_to_the_plugin_that_names_it(cx: &mut TestAppContext) {
    let pages = cx.update(|cx| super::init(cx).pages);
    let catalog = crate::shell_setup::plugin_catalog().expect("the application catalog");
    let nested: Vec<_> = pages
        .pages()
        .iter()
        .filter_map(|page| page.nested_under().map(|under| (page.id(), under)))
        .collect();
    for (page, (parent, anchor)) in &nested {
        assert_eq!(*parent, "plugins", "{page} is nested under Plugins");
        let plugin = catalog
            .plugins()
            .find(|plugin| plugin.id.as_str() == *anchor)
            .unwrap_or_else(|| panic!("{page} is anchored at a plugin, not {anchor}"));
        assert_eq!(plugin.settings_page, Some(*page));
    }
    for plugin in catalog
        .plugins()
        .filter(|plugin| plugin.settings_page.is_some())
    {
        let page = plugin.settings_page.unwrap();
        assert!(
            pages.pages().iter().any(|listed| listed.id() == page),
            "{} names {page}, which the settings catalog does not have",
            plugin.id
        );
        let anchored = nested.contains(&(page, ("plugins", plugin.id.as_str())));
        let top_level = nested.iter().all(|(nested_page, _)| *nested_page != page);
        assert!(
            anchored || top_level,
            "{} names {page}, which is nested under another plugin",
            plugin.id
        );
    }
    let ids: Vec<_> = pages.pages().iter().map(|page| page.id()).collect();
    let plugins = ids.iter().position(|id| *id == "plugins").unwrap();
    assert!(
        ids[..plugins].contains(&"keymap") && ids[..plugins].contains(&"explorer"),
        "Plugins comes after the other guests: {ids:?}"
    );
}

#[gpui_kit::test]
fn a_plugin_deep_link_opens_the_plugins_page_with_that_plugin_selected(cx: &mut TestAppContext) {
    let (handle, workspace, _) = window_over("", cx);
    dispatch(
        handle,
        OpenPlugins {
            plugin: Some("monitor".into()),
        },
        cx,
    );
    render(handle, cx);
    assert_eq!(selected_page(&workspace, cx), "plugins");
    assert_eq!(detail_label(handle, cx).as_deref(), Some("Monitor"));

    dispatch(handle, OpenPlugins { plugin: None }, cx);
    render(handle, cx);
    assert_eq!(
        detail_label(handle, cx).as_deref(),
        Some("Monitor"),
        "no plugin named keeps the selection"
    );
}

#[gpui_kit::test]
fn the_plugins_list_holds_the_plugins_and_no_core_entries(cx: &mut TestAppContext) {
    let (handle, _, _) = window_over("", cx);
    dispatch(handle, OpenPlugins { plugin: None }, cx);
    render(handle, cx);
    let catalog = crate::shell_setup::plugin_catalog().expect("the application catalog");
    cx.update_window(handle, |_, window, _| {
        let mut previous = None;
        for plugin in catalog.plugins() {
            let row = window.find(format!("plugin-row:{}", plugin.id));
            let top = row.bounds().origin.y;
            assert!(previous.is_none_or(|above| above < top), "catalog order");
            previous = Some(top);
        }
        for entry in catalog
            .entries()
            .filter(|entry| catalog.plugin_of(&entry.id).is_none())
        {
            let namespace = entry.id.as_str().split('.').next().unwrap();
            assert!(
                window.try_find(format!("plugin-row:{namespace}")).is_none(),
                "{namespace} is core and never listed"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_plugin_page_opens_from_its_gear_and_the_back_link_returns_to_the_plugin(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, _) = window_over("", cx);
    dispatch(
        handle,
        OpenPlugins {
            plugin: Some("monitor".into()),
        },
        cx,
    );
    render(handle, cx);
    cx.update_window(handle, |_, window, cx| window.click("plugin-page-link", cx))
        .unwrap();
    cx.run_until_parked();
    render(handle, cx);
    assert_eq!(selected_page(&workspace, cx), "monitor");
    cx.update_window(handle, |_, window, cx| {
        window.click("settings-back", cx);
    })
    .unwrap();
    render(handle, cx);
    assert_eq!(selected_page(&workspace, cx), "plugins");
    assert_eq!(detail_label(handle, cx).as_deref(), Some("Monitor"));

    dispatch(
        handle,
        OpenPluginPage {
            plugin: "projects".into(),
        },
        cx,
    );
    assert_eq!(selected_page(&workspace, cx), "workspaces");
    dispatch(
        handle,
        OpenPluginPage {
            plugin: "nobody".into(),
        },
        cx,
    );
    assert_eq!(
        selected_page(&workspace, cx),
        "workspaces",
        "an unknown plugin opens nothing"
    );
}

#[gpui_kit::test]
fn the_agent_row_links_to_the_top_level_ai_page(cx: &mut TestAppContext) {
    let (handle, workspace, _) = window_over("", cx);
    dispatch(
        handle,
        OpenPlugins {
            plugin: Some("agent".into()),
        },
        cx,
    );
    render(handle, cx);
    assert_eq!(detail_label(handle, cx).as_deref(), Some("AI Agents"));
    cx.update_window(handle, |_, window, cx| window.click("plugin-page-link", cx))
        .unwrap();
    cx.run_until_parked();
    render(handle, cx);
    assert_eq!(selected_page(&workspace, cx), "ai");
}

#[gpui_kit::test]
fn the_monitor_page_opens_and_saves_while_the_monitor_plugin_is_disabled(cx: &mut TestAppContext) {
    let (handle, workspace, settings) = window_over("[plugins]\ndisabled = [\"monitor\"]\n", cx);
    dispatch(handle, nocterm_monitor_ui::OpenMonitorSettings, cx);
    render(handle, cx);
    assert_eq!(selected_page(&workspace, cx), "monitor");
    let before = cx.update(|cx| {
        settings
            .setting::<nocterm_monitor::MonitorSettings>(cx)
            .enabled
    });
    cx.update_window(handle, |_, window, cx| window.click("monitor-enabled", cx))
        .unwrap();
    cx.run_until_parked();
    let after = cx.update(|cx| {
        settings
            .setting::<nocterm_monitor::MonitorSettings>(cx)
            .enabled
    });
    assert_ne!(before, after);
}

#[gpui_kit::test]
fn the_switch_on_the_plugins_page_stores_the_choice_in_the_plugins_section(
    cx: &mut TestAppContext,
) {
    let (handle, _, settings) = window_over("", cx);
    dispatch(
        handle,
        OpenPlugins {
            plugin: Some("monitor".into()),
        },
        cx,
    );
    render(handle, cx);
    cx.update_window(handle, |_, window, cx| window.click("plugin-enabled", cx))
        .unwrap();
    cx.run_until_parked();
    let disabled = cx.update(|cx| {
        settings
            .setting::<nocterm_plugin_enablement::PluginEnablement>(cx)
            .disabled
            .clone()
    });
    assert_eq!(disabled, [nocterm_plugin_model::PluginId::new("monitor")]);

    dispatch(
        handle,
        OpenPlugins {
            plugin: Some("agent".into()),
        },
        cx,
    );
    render(handle, cx);
    cx.update_window(handle, |_, window, cx| window.click("plugin-enabled", cx))
        .unwrap();
    cx.run_until_parked();
    let disabled = cx.update(|cx| {
        settings
            .setting::<nocterm_plugin_enablement::PluginEnablement>(cx)
            .disabled
            .len()
    });
    assert_eq!(disabled, 1, "a required plugin stays enabled");
}
