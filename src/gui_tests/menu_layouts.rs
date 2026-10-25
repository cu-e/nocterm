//! The Window menu's Layouts and Panels submenus follow the selected layout and the panels.
use super::fixture;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Menu, MenuItem, TestAppContext,
    test::TestWindowExt as _,
};
use nocterm_workspace::Workspace;

/// What the user sees of one submenu of the Window menu: label, checked, disabled. A separator
/// has the label `-`.
fn entries(menus: &[Menu], submenu: &str) -> Vec<(String, bool, bool)> {
    let window = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "Window")
        .expect("the Window menu");
    let menu = window
        .items
        .iter()
        .find_map(|item| match item {
            MenuItem::Submenu(menu) if menu.name.as_ref() == submenu => Some(menu),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the Window menu has a {submenu} submenu"));
    menu.items
        .iter()
        .map(|item| match item {
            MenuItem::Action {
                name,
                checked,
                disabled,
                ..
            } => (name.to_string(), *checked, *disabled),
            _ => ("-".to_owned(), false, false),
        })
        .collect()
}

fn window_entries(
    handle: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    submenu: &str,
    cx: &mut TestAppContext,
) -> Vec<(String, bool, bool)> {
    cx.update_window(handle, |_, window, cx| {
        let menus = crate::app_menus::build(workspace.read(cx), window, cx);
        entries(&menus, submenu)
    })
    .unwrap()
}

fn dispatch(handle: AnyWindowHandle, action: impl gpui_kit::Action, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(action), cx)
    })
    .unwrap();
    cx.run_until_parked();
}

fn checked_labels(entries: &[(String, bool, bool)]) -> Vec<&str> {
    entries
        .iter()
        .filter(|(_, checked, _)| *checked)
        .map(|(label, _, _)| label.as_str())
        .collect()
}

#[gpui_kit::test]
fn the_layouts_submenu_checks_the_selected_layout_and_follows_a_switch(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    let labels: Vec<_> = window_entries(handle, &workspace, "Layouts", cx)
        .into_iter()
        .map(|(label, _, _)| label)
        .collect();
    assert_eq!(
        labels,
        [
            "Default",
            "Swapped sides",
            "-",
            "Next Layout",
            "Previous Layout",
            "Switch Layout…",
            "Customize…"
        ]
    );
    assert_eq!(
        checked_labels(&window_entries(handle, &workspace, "Layouts", cx)),
        ["Default"]
    );
    dispatch(
        handle,
        nocterm_workspace::SwitchLayout {
            id: "swapped".into(),
        },
        cx,
    );
    assert_eq!(
        checked_labels(&window_entries(handle, &workspace, "Layouts", cx)),
        ["Swapped sides"]
    );
    dispatch(handle, nocterm_workspace::NextLayout, cx);
    assert_eq!(
        checked_labels(&window_entries(handle, &workspace, "Layouts", cx)),
        ["Default"]
    );
}

#[gpui_kit::test]
fn customize_in_the_layouts_submenu_opens_the_editor_in_appearance(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    let customize = window_entries(handle, &workspace, "Layouts", cx)
        .into_iter()
        .find(|(label, ..)| label == "Customize…")
        .expect("the Layouts submenu offers Customize…");
    assert!(!customize.2, "it is available in a workspace window");

    let action = cx
        .update_window(handle, |_, window, cx| {
            let menus = crate::app_menus::build(workspace.read(cx), window, cx);
            menus
                .iter()
                .filter(|menu| menu.name.as_ref() == "Window")
                .flat_map(|menu| &menu.items)
                .filter_map(|item| match item {
                    MenuItem::Submenu(menu) if menu.name.as_ref() == "Layouts" => Some(menu),
                    _ => None,
                })
                .flat_map(|menu| &menu.items)
                .find_map(|item| match item {
                    MenuItem::Action { name, action, .. } if name.as_ref() == "Customize…" => {
                        Some(action.boxed_clone())
                    }
                    _ => None,
                })
                .expect("the item carries its action")
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| window.dispatch_action(action, cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("layout-editor".to_owned()).is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_panels_submenu_lists_the_placed_panels_and_toggles_them(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    let panels = window_entries(handle, &workspace, "Panels", cx);
    let labels: Vec<_> = panels.iter().map(|(label, _, _)| label.as_str()).collect();
    assert!(labels.contains(&"AI Agents"), "{labels:?}");
    assert!(labels.contains(&"-"), "groups are separated: {labels:?}");
    assert!(!checked_labels(&panels).contains(&"AI Agents"));
    dispatch(
        handle,
        nocterm_workspace::TogglePanel {
            id: "agent.chats".into(),
        },
        cx,
    );
    assert!(
        checked_labels(&window_entries(handle, &workspace, "Panels", cx)).contains(&"AI Agents")
    );
    dispatch(
        handle,
        nocterm_workspace::TogglePanel {
            id: "agent.chats".into(),
        },
        cx,
    );
    assert!(
        !checked_labels(&window_entries(handle, &workspace, "Panels", cx)).contains(&"AI Agents")
    );
}

#[gpui_kit::test]
fn toggling_a_panel_the_catalog_does_not_list_does_nothing(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    let before = window_entries(handle, &workspace, "Panels", cx);
    dispatch(
        handle,
        nocterm_workspace::TogglePanel {
            id: "nobody.nothing".into(),
        },
        cx,
    );
    assert_eq!(window_entries(handle, &workspace, "Panels", cx), before);
}

/// A window over a local host whose requests the test can read, under the given settings.
pub(super) fn window_on_local_host(
    layouts: &str,
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    nocterm_session::testing::FakeExec,
    nocterm_store::SettingsHandle,
    Entity<Workspace>,
) {
    let window = open_local_window(layouts, cx);
    (
        window.handle,
        window.exec,
        window.settings,
        window.workspace,
    )
}

/// What [`open_local_window`] opened, with the composed application it opened over.
pub(super) struct LocalWindow {
    pub(super) handle: AnyWindowHandle,
    pub(super) exec: nocterm_session::testing::FakeExec,
    pub(super) settings: nocterm_store::SettingsHandle,
    pub(super) workspace: Entity<Workspace>,
    /// The composed application, for the tests that read its user plugin shell.
    #[cfg(feature = "user-plugins")]
    pub(super) booted: crate::bootstrap::Booted,
}

pub(super) fn open_local_window(layouts: &str, cx: &mut TestAppContext) -> LocalWindow {
    open_local_window_in(layouts, crate::shell_setup::UserPluginMode::default(), cx)
}

/// [`open_local_window`] with the user plugins composed in `mode`.
pub(super) fn open_local_window_in(
    layouts: &str,
    mode: crate::shell_setup::UserPluginMode,
    cx: &mut TestAppContext,
) -> LocalWindow {
    use std::sync::Arc;
    let (handle, exec, settings, workspace, booted) = cx.update(|cx| {
        let document = nocterm_settings::SettingsDocument::from_table(
            layouts.parse().expect("the layouts are toml"),
        );
        let transport = Arc::new(super::MockTransport::default());
        let local = {
            let local = transport.clone();
            nocterm_terminal::LocalSessions::new(
                Arc::new(move |_| local.clone()),
                "/bin/bash".into(),
            )
        };
        let booted = super::boot_with_ports_and_probe(
            tempfile::tempdir().unwrap(),
            nocterm_store::SettingsConfig::in_memory(document),
            (transport, local),
            false,
            None,
            None,
            super::unsupported_sandbox(),
            mode,
            cx,
        );
        let (handle, workspace) = super::open_workspace(&booted, cx);
        (
            handle,
            cx.global::<super::FixtureLocalHost>().0.clone(),
            booted.settings_deps().settings().clone(),
            workspace,
            booted,
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    #[cfg(not(feature = "user-plugins"))]
    drop(booted);
    LocalWindow {
        handle,
        exec,
        settings,
        workspace,
        #[cfg(feature = "user-plugins")]
        booted,
    }
}

#[gpui_kit::test]
fn a_monitor_the_layout_leaves_in_the_tray_never_reaches_the_host(cx: &mut TestAppContext) {
    // The monitor feeds its sampling script to a shell; other features run their own commands.
    let samples = |exec: &nocterm_session::testing::FakeExec| {
        exec.ran().iter().any(|command| command == "sh -s")
    };
    let (_, exec, ..) = window_on_local_host("", cx);
    assert!(samples(&exec), "the placed monitor samples the host");
    let quiet = "[layouts]\nactive = \"p1\"\n[[layouts.presets]]\nid = \"p1\"\n\
                 name = \"Quiet\"\ntray = [\"monitor.summary\"]\n";
    let (_, exec, ..) = window_on_local_host(quiet, cx);
    assert!(!samples(&exec), "ran {:?}", exec.ran());
}

#[gpui_kit::test]
fn a_disabled_monitor_plugin_never_reaches_the_host(cx: &mut TestAppContext) {
    let samples = |exec: &nocterm_session::testing::FakeExec| {
        exec.ran().iter().any(|command| command == "sh -s")
    };
    let (_, exec, ..) = window_on_local_host("[plugins]\ndisabled = [\"monitor\"]\n", cx);
    assert!(!samples(&exec), "ran {:?}", exec.ran());
}

#[gpui_kit::test]
fn the_panels_submenu_omits_a_panel_of_a_disabled_plugin(cx: &mut TestAppContext) {
    let (handle, workspace, ..) = {
        let document = nocterm_settings::SettingsDocument::from_table(
            "[plugins]\ndisabled = [\"snippets\"]\n"
                .parse()
                .expect("the settings are toml"),
        );
        super::fixture_with_settings(
            cx,
            false,
            None,
            false,
            Some(nocterm_store::SettingsConfig::in_memory(document)),
        )
    };
    let panels = window_entries(handle, &workspace, "Panels", cx);
    let labels: Vec<_> = panels.iter().map(|(label, _, _)| label.as_str()).collect();
    assert!(labels.contains(&"AI Agents"), "{labels:?}");
    assert!(!labels.contains(&"Snippets"), "{labels:?}");

    let (handle, workspace, ..) = fixture(cx);
    let panels = window_entries(handle, &workspace, "Panels", cx);
    let labels: Vec<_> = panels.iter().map(|(label, _, _)| label.as_str()).collect();
    assert!(labels.contains(&"Snippets"), "{labels:?}");
}
