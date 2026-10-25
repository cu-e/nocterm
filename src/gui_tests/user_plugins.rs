//! User plugins in the composed application: an installed package reaches the window as a
//! framed widget or panel, follows enabling and placement, and leaves with its plugin.
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, TestAppContext, VisualTestContext,
    test::TestWindowExt as _,
};
use nocterm_plugin_model::{ContributionId, PluginId};
use nocterm_workspace::Workspace;

use super::{FixtureUserMode, FixtureUserPlugins, menu_layouts::window_on_local_host};
use crate::shell_setup::{UserPluginMode, UserShell};

const WIDGET: &str = "user-example-clock.face";
const PANEL: &str = "user-example-notes.board";

/// The clock widget on the bar's right and the notes panel in the navigation group.
const PLACED: &str = r#"
[layouts]
active = "p1"
[[layouts.presets]]
id = "p1"
name = "With plugins"
[layouts.presets.bar]
left = [{ group = "navigation", primary = true, members = ["files.explorer", "user-example-notes.board"] }]
right = [{ item = "workspace.notifications" }, { item = "user-example-clock.face" }]
"#;

struct Window {
    handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    shell: UserShell,
}

fn window(cx: &mut TestAppContext) -> Window {
    let (handle, _, _, workspace) = window_on_local_host(PLACED, cx);
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1400.), gpui_kit::px(800.)));
        window.render_frame(cx);
    })
    .unwrap();
    let shell = cx.update(|cx| cx.global::<FixtureUserPlugins>().0.clone());
    Window {
        handle,
        workspace,
        shell,
    }
}

fn example(name: &str) -> String {
    format!(
        "{}/examples/plugins/{name}.toml",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn install(window: &Window, name: &str, placeable: bool, cx: &mut TestAppContext) -> PluginId {
    let id = cx.update(|cx| {
        window
            .shell
            .host()
            .expect("the shell loads user plugins")
            .update(cx, |host, cx| {
                let package = host
                    .read_package(&example(name))
                    .expect("the example reads");
                let id = host
                    .install(package, &[], cx)
                    .expect("the example installs");
                host.set_placeable(&id, placeable, cx)
                    .expect("it is installed");
                id
            })
    });
    cx.run_until_parked();
    id
}

fn change(
    window: &Window,
    cx: &mut TestAppContext,
    act: impl FnOnce(
        &mut nocterm_plugin_host::PluginHost,
        &mut gpui_kit::Context<nocterm_plugin_host::PluginHost>,
    ),
) {
    cx.update(|cx| {
        window
            .shell
            .host()
            .expect("the shell loads user plugins")
            .update(cx, act)
    });
    cx.run_until_parked();
}

fn registered(window: &Window, id: &str, cx: &mut TestAppContext) -> bool {
    cx.update(|cx| {
        window
            .workspace
            .read(cx)
            .is_registered(&ContributionId::owned(id))
    })
}

fn present(window: &Window, selector: &'static str, cx: &mut TestAppContext) -> bool {
    // The bar measures its entries as it draws and settles one frame later.
    for _ in 0..3 {
        cx.update_window(window.handle, |_, w, cx| w.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
    VisualTestContext::from_window(window.handle, cx)
        .into_mut()
        .debug_bounds(selector)
        .is_some()
}

/// Opens the panel the way the user does, from the menu, and says whether it shows.
fn open_panel(window: &Window, cx: &mut TestAppContext) -> bool {
    let id = ContributionId::owned(PANEL);
    cx.update_window(window.handle, |_, w, cx| {
        window
            .workspace
            .update(cx, |workspace, cx| workspace.open_panel(&id, w, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| window.workspace.read(cx).panel_is_open(&id))
}

#[gpui_kit::test]
fn an_installed_widget_is_drawn_in_the_bar_with_the_plugin_marker(cx: &mut TestAppContext) {
    let window = window(cx);
    assert!(!present(&window, "plugin-marker", cx));

    install(&window, "clock", true, cx);

    assert!(registered(&window, WIDGET, cx));
    assert!(present(&window, "plugin-marker", cx));
}

#[gpui_kit::test]
fn a_widget_of_a_developer_session_carries_the_dev_chip(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(FixtureUserMode(UserPluginMode::Dev("clock.toml".into())));
    });
    let window = window(cx);

    install(&window, "clock", true, cx);

    assert!(present(&window, "plugin-marker", cx));
    assert!(present(&window, "plugin-dev", cx));
}

#[gpui_kit::test]
fn a_widget_of_an_installation_carries_no_dev_chip(cx: &mut TestAppContext) {
    let window = window(cx);

    install(&window, "clock", true, cx);

    assert!(present(&window, "plugin-marker", cx));
    assert!(!present(&window, "plugin-dev", cx));
}

#[gpui_kit::test]
fn a_package_not_yet_placeable_registers_nothing_it_can_draw(cx: &mut TestAppContext) {
    let window = window(cx);

    install(&window, "clock", false, cx);

    assert!(!present(&window, "plugin-marker", cx));
}

#[gpui_kit::test]
fn disabling_a_plugin_removes_its_widget_and_enabling_brings_it_back(cx: &mut TestAppContext) {
    let window = window(cx);
    let id = install(&window, "clock", true, cx);

    change(&window, cx, |host, cx| {
        host.set_enabled(&id, false, cx).unwrap()
    });
    assert!(!present(&window, "plugin-marker", cx));

    change(&window, cx, |host, cx| {
        host.set_enabled(&id, true, cx).unwrap()
    });
    assert!(present(&window, "plugin-marker", cx));
}

#[gpui_kit::test]
fn removing_a_plugin_withdraws_its_contributions(cx: &mut TestAppContext) {
    let window = window(cx);
    let id = install(&window, "clock", true, cx);

    change(&window, cx, |host, cx| {
        host.uninstall(&id, true, cx).unwrap()
    });

    assert!(!registered(&window, WIDGET, cx));
    assert!(!present(&window, "plugin-marker", cx));
}

#[gpui_kit::test]
fn an_opened_user_panel_shows_the_plugin_frame_and_is_named_in_the_menu(cx: &mut TestAppContext) {
    let window = window(cx);
    install(&window, "notes", true, cx);

    assert!(open_panel(&window, cx));

    assert!(present(&window, "plugin-marker", cx));
    assert!(present(&window, "plugin-label", cx));
    let labels: Vec<String> = cx.update(|cx| {
        let workspace = window.workspace.read(cx);
        workspace
            .panel_menu()
            .concat()
            .iter()
            .map(|panel| panel.label())
            .collect()
    });
    assert!(
        labels
            .iter()
            .any(|label| label == "Plugin \u{b7} Example Notes \u{2014} Notes board"),
        "{labels:?}"
    );
}

#[gpui_kit::test]
fn a_panel_of_a_package_that_may_not_be_placed_does_not_open(cx: &mut TestAppContext) {
    let window = window(cx);
    install(&window, "notes", false, cx);

    assert!(!open_panel(&window, cx));
    assert!(!present(&window, "plugin-marker", cx));
}

#[gpui_kit::test]
fn the_panels_menu_lists_a_user_panel_by_its_plugin_in_a_user_plugins_submenu(
    cx: &mut TestAppContext,
) {
    let window = window(cx);
    install(&window, "notes", true, cx);

    let menus = cx
        .update_window(window.handle, |_, w, cx| {
            crate::app_menus::build(window.workspace.read(cx), w, cx)
        })
        .unwrap();

    let panels = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "Window")
        .and_then(|menu| {
            menu.items.iter().find_map(|item| match item {
                gpui_kit::MenuItem::Submenu(menu) if menu.name.as_ref() == "Panels" => Some(menu),
                _ => None,
            })
        })
        .expect("the Window menu has a Panels submenu");
    let user = panels
        .items
        .iter()
        .find_map(|item| match item {
            gpui_kit::MenuItem::Submenu(menu) if menu.name.as_ref() == "User plugins" => Some(menu),
            _ => None,
        })
        .expect("the Panels menu has a User plugins submenu");
    let labels: Vec<_> = user
        .items
        .iter()
        .filter_map(|item| match item {
            gpui_kit::MenuItem::Action { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["Plugin \u{b7} Example Notes \u{2014} Notes board"]);
    assert!(
        !panels.items.iter().any(|item| matches!(
            item,
            gpui_kit::MenuItem::Action { name, .. } if name.contains("Notes board")
        )),
        "the shell's own list does not mix the plugin in"
    );
}

#[gpui_kit::test]
fn a_user_plugin_setting_is_kept_by_the_host_and_never_reaches_the_settings_file(
    cx: &mut TestAppContext,
) {
    let (_, _, settings, workspace) = window_on_local_host("", cx);
    let _ = workspace;
    let shell = cx.update(|cx| cx.global::<FixtureUserPlugins>().0.clone());
    let id = cx.update(|cx| {
        shell
            .host()
            .expect("the shell loads user plugins")
            .update(cx, |host, cx| {
                let package = host.read_package(&example("clock")).unwrap();
                host.install(package, &[], cx).unwrap()
            })
    });
    let before = cx.update(|cx| settings.document(cx).to_table());
    let directory = shell.directory(nocterm_ui::PluginSettingsDirectory::new());

    let (schema, source) = cx
        .update(|cx| directory.get(&id, cx))
        .expect("the installed plugin has settings");
    assert!(schema.defaults().get("label").is_some());
    let task = cx.update(|cx| {
        source.write(
            "label",
            nocterm_plugin_model::FieldEdit::Set(nocterm_plugin_model::SettingValue::Text(
                "Tokyo".into(),
            )),
            cx,
        )
    });
    cx.run_until_parked();
    cx.foreground_executor().block_test(task).unwrap();

    let stored = cx.update(|cx| source.values(cx).get("label").cloned());
    assert_eq!(
        stored,
        Some(nocterm_plugin_model::SettingValue::Text("Tokyo".into()))
    );
    assert_eq!(cx.update(|cx| settings.document(cx).to_table()), before);
}

#[gpui_kit::test]
fn a_user_id_resolves_to_the_user_source_and_never_to_a_system_one(cx: &mut TestAppContext) {
    let (_, _, settings, _) = window_on_local_host("", cx);
    let shell = cx.update(|cx| cx.global::<FixtureUserPlugins>().0.clone());
    let system = nocterm_ui::PluginSettingsDirectory::new().with_system(
        &nocterm_workspace::clock::PLUGIN,
        nocterm_workspace::clock::plugin_settings(settings),
    );
    let directory = shell.directory(system);
    let clock = nocterm_workspace::clock::PLUGIN.id.clone();
    let installed = cx.update(|cx| {
        shell
            .host()
            .expect("the shell loads user plugins")
            .update(cx, |host, cx| {
                let package = host.read_package(&example("clock")).unwrap();
                host.install(package, &[], cx).unwrap()
            })
    });

    let system_schema = cx
        .update(|cx| directory.get(&clock, cx))
        .map(|(schema, _)| schema);
    let user_schema = cx
        .update(|cx| directory.get(&installed, cx))
        .map(|(schema, _)| schema);

    let system_schema = system_schema.expect("the system list is not empty");
    let user_schema = user_schema.expect("the installed plugin resolves to the user source");
    assert!(system_schema.defaults().get("label").is_none());
    assert!(user_schema.defaults().get("label").is_some());
    assert!(
        cx.update(|cx| directory.get(&PluginId::owned("user-example-nobody"), cx))
            .is_none(),
        "a user id nobody installed does not fall back to the system list"
    );
}

#[gpui_kit::test]
fn the_bar_editor_is_told_the_hosts_facts_of_an_installed_plugin(cx: &mut TestAppContext) {
    let window = window(cx);
    let id = install(&window, "notes", true, cx);
    let facts = window.shell.facts().expect("the build has user plugins");

    let known = cx
        .update(|cx| facts.facts(&id, cx))
        .expect("the plugin is installed");

    assert_eq!(known.publisher, "example");
    assert_eq!(known.fingerprint.len(), 16);
    assert_eq!(
        known.capabilities,
        [
            "See how busy this computer is.",
            "See how many file transfers are running or failed."
        ]
    );
    assert!(
        cx.update(|cx| facts.facts(&PluginId::owned("user-example-nobody"), cx))
            .is_none()
    );
}
