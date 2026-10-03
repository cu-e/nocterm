//! GPUI dispatch and session lifecycle regressions using a scripted transport.

use std::sync::{Arc, Mutex};

use futures::FutureExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, InputEvent as _, TestAppContext,
    WindowOptions, test::TestWindowExt as _,
};
use nocterm_session::{
    CloseReason, ConnectRequest, Event, Prompt, Reply, Secret, SecretRequest, Session,
    SessionDriver, Transport,
};
use nocterm_terminal::{TerminalView, open_session};
use nocterm_workspace::{SessionSpec, Workspace};

#[derive(Default)]
struct MockTransport {
    drivers: Mutex<Vec<Arc<SessionDriver>>>,
}

struct FixtureDirectory {
    _directory: tempfile::TempDir,
}
impl gpui_kit::Global for FixtureDirectory {}

impl Transport for MockTransport {
    fn open(&self, _: ConnectRequest) -> Session {
        let (session, driver) = nocterm_session::channel(None);
        self.drivers.lock().unwrap().push(Arc::new(driver));
        session
    }
}

fn spec(host: &str) -> SessionSpec {
    SessionSpec {
        options: Default::default(),
        title: host.to_owned().into(),
        target: nocterm_session::Target::new("test", host, 22),
        auth: nocterm_session::Auth::Auto,
        launch: None,
        credential: None,
    }
}

fn fixture(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<TerminalView>,
    Arc<MockTransport>,
) {
    fixture_with_vault(cx, false)
}

fn fixture_with_vault(
    cx: &mut TestAppContext,
    vault_ready: bool,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<TerminalView>,
    Arc<MockTransport>,
) {
    let transport = Arc::new(MockTransport::default());
    let (window, workspace, terminal) = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        let directory = tempfile::tempdir().unwrap();
        let mut settings = nocterm_settings::Settings::default();
        settings.local.cwd = Some(directory.path().to_string_lossy().into_owned());
        let vault_path = directory.path().join("vault.bin");
        let paths = nocterm_core::Paths::rooted_at(directory.path());
        cx.set_global(FixtureDirectory {
            _directory: directory,
        });
        nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            nocterm_ui::SettingsStore::in_memory(settings),
            cx,
        );
        if vault_ready {
            nocterm_vault_ui::init(vault_path, cx).unwrap();
        }
        nocterm_terminal::init(transport.clone(), cx);
        nocterm_connections::init(None, cx);
        crate::keymap::load(cx);
        super::application::register(paths, vault_ready, cx);
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    workspace.set_session_opener(open_session);
                    nocterm_connections::register(&mut workspace, window, cx);
                    super::register_settings(&mut workspace, vault_ready);
                    nocterm_files::register(&mut workspace, cx);
                    workspace.set_menu_builder(super::app_menus::build, window, cx);
                    workspace
                })
            })
            .unwrap();
        let terminal = window
            .update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.open_session(spec("first.test"), window, cx)
                });
                workspace.read(cx).find_item::<TerminalView>().unwrap()
            })
            .unwrap();
        (window, workspace, terminal)
    });
    emit(cx, &transport, 0, Event::Connected);
    cx.update_window(window, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        assert!(
            workspace
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx),
            "terminal dispatch must be inside Workspace"
        );
        assert!(terminal.read(cx).terminal().read(cx).is_connected());
        assert!(terminal.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    (window, workspace, terminal, transport)
}

fn emit(cx: &mut TestAppContext, transport: &MockTransport, index: usize, event: Event) {
    let driver = transport.drivers.lock().unwrap()[index].clone();
    cx.background_executor
        .spawn(async move {
            assert!(driver.emit(event).await);
        })
        .detach();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn titlebar_find_and_settings_keep_their_field_focus(cx: &mut TestAppContext) {
    let (handle, workspace, terminal, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for (index, label) in ["Session", "Edit", "Search", "Window", "Help"]
            .iter()
            .enumerate()
        {
            assert_eq!(
                window
                    .within("app-menu-bar")
                    .within(index)
                    .find("menu")
                    .label(),
                Some(*label)
            );
        }
        window
            .within("app-menu-bar")
            .within(2usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("find-previous").is_some());
        assert!(terminal.read(cx).focus_handle(cx).is_focused(window));
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").hover(6usize, cx);
        window.render_frame(cx);
        window
            .within("submenu")
            .within("popup-menu")
            .click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let settings = workspace
            .read(cx)
            .find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap();
        assert!(settings.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn titlebar_about_and_new_window_use_application_actions(cx: &mut TestAppContext) {
    let (first, workspace, _, transport) = fixture(cx);
    cx.update_window(first, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(4usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(first, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("about-close").is_some());
        window.click("about-close", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(first, |_, window, cx| {
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(1usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let second = cx.update(|cx| {
        let windows = cx.windows();
        assert_eq!(windows.len(), 2);
        assert_eq!(
            transport.drivers.lock().unwrap().len(),
            1,
            "opening a window does not start another connection"
        );
        assert!(workspace.read(cx).active_session(cx).unwrap().connected);
        *windows.iter().find(|window| **window != first).unwrap()
    });
    cx.update_window(second, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_workspace::CloseWindow), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(cx.windows(), vec![first]);
        assert!(workspace.read(cx).active_session(cx).unwrap().connected);
    });
}

#[gpui_kit::test]
fn titlebar_commands_follow_bottom_screen_focus_while_find_remains_open(cx: &mut TestAppContext) {
    let (handle, workspace, central, transport) = fixture(cx);
    let bottom = cx
        .update_window(handle, |_, window, cx| {
            let local_transport = transport.clone();
            nocterm_terminal::init_local(Arc::new(move |_| local_transport.clone()), cx);
            let bottom = cx.new(|cx| TerminalView::new_local(window, cx));
            workspace.update(cx, |workspace, cx| {
                workspace.set_local_terminal(bottom.clone(), window, cx);
            });
            bottom
        })
        .unwrap();
    cx.simulate_window_resize(
        handle,
        gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(800.)),
    );
    emit(cx, &transport, 1, Event::Connected);
    let grid_focus = bottom.read_with(cx, |bottom, cx| bottom.focus_handle(cx));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(grid_focus.is_focused(window));
        window
            .within("app-menu-bar")
            .within(2usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("find-previous").is_some());
        assert!(bottom.read(cx).focus_handle(cx).is_focused(window));
        // TerminalElement is a canvas rather than an observed component. Send
        // native pointer events inside the bottom grid, above the footer.
        let position = gpui_kit::point(
            window.viewport_size().width - gpui_kit::px(100.),
            window.viewport_size().height - gpui_kit::px(70.),
        );
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                button: gpui_kit::MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(
            grid_focus.is_focused(window),
            "the click must actually focus the bottom terminal screen"
        );
        assert!(
            window.try_find("find-previous").is_some(),
            "the Find bar remains mounted"
        );
        window
            .within("app-menu-bar")
            .within(1usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(9usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("Local terminal"),
            "Copy Connection Name must target the focused bottom screen despite its open Find bar"
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(3usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        assert!(!bottom.read(cx).terminal().read(cx).is_connected());
        assert!(
            central.read(cx).terminal().read(cx).is_connected(),
            "Disconnect from the bottom screen must preserve the central remote session"
        );
    });
}

#[gpui_kit::test]
fn titlebar_close_window_retires_connected_session_and_releases_its_models(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, terminal, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    let workspace_weak = workspace.downgrade();
    let terminal_weak = terminal.downgrade();
    // Keep only the references a running application owns, so closing the
    // window must dispose its entities and retire its session driver.
    drop(workspace);
    drop(terminal);
    cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window
            .within("app-menu-bar")
            .within(0usize)
            .click("menu", cx);
        window.render_frame(cx);
        window.within("popup-menu").click(10usize, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(cx.windows().is_empty());
        assert!(
            workspace_weak.upgrade().is_none(),
            "the closed window must release its Workspace"
        );
        assert!(
            terminal_weak.upgrade().is_none(),
            "the closed window must release its TerminalView"
        );
    });
    assert!(
        driver.closed().now_or_never().is_some(),
        "closing the window must retire the connected session"
    );
}

#[gpui_kit::test]
fn tab_keys_reach_shell_without_moving_focus(cx: &mut TestAppContext) {
    let (handle, _, terminal, transport) = fixture(cx);
    let driver = transport.drivers.lock().unwrap()[0].clone();
    // Ignore PTY sizing during the initial layout.
    while driver.next_command().now_or_never().flatten().is_some() {}
    cx.update_window(handle, |_, window, cx| {
        for key in ["tab", "shift-tab"] {
            window.press(key, cx);
            window.render_frame(cx);
            assert!(terminal.read(cx).focus_handle(cx).is_focused(window));
        }
    })
    .unwrap();
    let mut inputs = Vec::new();
    while let Some(Some(command)) = driver.next_command().now_or_never() {
        if let nocterm_session::Command::Input(bytes) = command {
            inputs.push(bytes);
        }
    }
    assert_eq!(inputs, [b"\t".to_vec(), b"\x1b[Z".to_vec()]);
}

#[gpui_kit::test]
fn vault_action_opens_the_vault_page_in_the_single_settings_item(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture_with_vault(cx, true);
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::OpenSettings), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let original = workspace.read_with(cx, |w, _| {
        w.find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap()
            .entity_id()
    });
    cx.update_window(handle, |_, window, cx| {
        assert!(window.try_find("vault-submit").is_none());
        window.dispatch_action(Box::new(nocterm_workspace::OpenVault), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("vault-submit").visible());
        window.dispatch_action(Box::new(nocterm_workspace::OpenVault), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("vault-submit").visible());
    })
    .unwrap();
    assert_eq!(
        workspace.read_with(cx, |w, _| w
            .find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap()
            .entity_id()),
        original
    );
}

#[gpui_kit::test]
fn secret_prompt_keeps_tab_navigation(cx: &mut TestAppContext) {
    let (handle, _, terminal, transport) = fixture(cx);
    let (reply, _answer) = Reply::<Option<Secret>>::channel();
    emit(
        cx,
        &transport,
        0,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Password {
                target: spec("first.test").target,
                retry: false,
            },
            reply,
        }),
    );
    let driver = transport.drivers.lock().unwrap()[0].clone();
    while driver.next_command().now_or_never().flatten().is_some() {}
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let input_focus = terminal.read(cx).focus_handle(cx);
        assert!(input_focus.is_focused(window));
        window.press("tab", cx);
        window.render_frame(cx);
        assert!(!input_focus.is_focused(window));
        window.press("shift-tab", cx);
        window.render_frame(cx);
        assert!(input_focus.is_focused(window));
    })
    .unwrap();
    while let Some(Some(command)) = driver.next_command().now_or_never() {
        assert!(!matches!(command, nocterm_session::Command::Input(_)));
    }
}

#[gpui_kit::test]
fn new_tab_shortcut_from_connected_terminal_opens_menu(cx: &mut TestAppContext) {
    let (handle, _, _, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-t"
            } else {
                "ctrl-shift-t"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("menu-new-connection").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn settings_shortcut_from_connected_terminal_opens_settings(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-,"
            } else {
                "ctrl-,"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).items().count(), 2);
        let settings = workspace
            .read(cx)
            .find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap();
        assert!(settings.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn close_shortcut_from_connected_terminal_closes_tab(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-shift-w"
            },
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |workspace, _| workspace.items().count()),
        0
    );
}

#[gpui_kit::test]
fn delayed_secret_prompt_preserves_other_tab_focus_and_restores_input_on_activation(
    cx: &mut TestAppContext,
) {
    let (handle, workspace, first, transport) = fixture(cx);
    let second = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.open_session(spec("second.test"), window, cx)
            });
            workspace
                .read(cx)
                .active_item()
                .unwrap()
                .view()
                .downcast::<TerminalView>()
                .unwrap()
        })
        .unwrap();
    emit(cx, &transport, 1, Event::Connected);
    let (reply, mut answer) = Reply::<Option<Secret>>::channel();
    emit(
        cx,
        &transport,
        0,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Interactive {
                prompt: "Code:".into(),
                echo: true,
            },
            reply,
        }),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            second.read(cx).focus_handle(cx).is_focused(window),
            "an inactive tab must not steal focus for its prompt"
        );
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(first.entity_id(), window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(first.read(cx).focus_handle(cx).is_focused(window));
        window.input("123456", cx);
        window.click("secret-submit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        answer.try_recv().unwrap().unwrap().unwrap().expose(),
        "123456",
        "activation must focus the secret input instead of sending text to the terminal"
    );

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_item_by_id(second.entity_id(), window, cx)
        });
    })
    .unwrap();
    let (reply, _answer) = Reply::<Option<Secret>>::channel();
    emit(
        cx,
        &transport,
        0,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Interactive {
                prompt: "Again:".into(),
                echo: true,
            },
            reply,
        }),
    );
    emit(
        cx,
        &transport,
        0,
        Event::Closed(CloseReason::Exited(Some(0))),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            second.read(cx).focus_handle(cx).is_focused(window),
            "removing an inactive prompt must not steal focus"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn active_secret_prompt_accepts_text_and_releases_focus_after_submit(cx: &mut TestAppContext) {
    let (handle, _, terminal, transport) = fixture(cx);
    let grid_focus = terminal.read_with(cx, |terminal, cx| terminal.focus_handle(cx));
    let (reply, mut answer) = Reply::<Option<Secret>>::channel();
    emit(
        cx,
        &transport,
        0,
        Event::Prompt(Prompt::Secret {
            request: SecretRequest::Interactive {
                prompt: "Code:".into(),
                echo: true,
            },
            reply,
        }),
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let prompt_focus = terminal.read(cx).focus_handle(cx);
        assert_ne!(prompt_focus, grid_focus);
        assert!(prompt_focus.is_focused(window));
        window.input("654321", cx);
        window.click("secret-submit", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        answer.try_recv().unwrap().unwrap().unwrap().expose(),
        "654321"
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(grid_focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn reconnect_button_works_when_file_sidebar_has_focus(cx: &mut TestAppContext) {
    let (handle, workspace, terminal, transport) = fixture(cx);
    emit(
        cx,
        &transport,
        0,
        Event::Closed(CloseReason::Exited(Some(0))),
    );
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.activate_panel_of::<nocterm_files::FilesPanel>(window, cx)
        });
        window.render_frame(cx);
        assert!(!terminal.read(cx).focus_handle(cx).is_focused(window));
        window.click("reconnect", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            transport.drivers.lock().unwrap().len(),
            2,
            "one click must reopen the session even from sidebar focus"
        );
        assert!(terminal.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}
