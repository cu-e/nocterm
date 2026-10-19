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
use nocterm_terminal::TerminalView;
use nocterm_workspace::{SessionSpec, Workspace};

mod agent_installation;
mod boot;
mod chat_history;
use boot::{
    boot, boot_with_files, boot_with_ports, boot_with_ports_and_probe, seed_native_ancestors,
};
mod keymap;
mod menu_tabs;
mod program_opening;
mod shortcuts;
mod snippets;
mod terminal_events;
mod titlebar;
mod vault;

#[derive(Default)]
struct MockTransport {
    drivers: Mutex<Vec<Arc<SessionDriver>>>,
}

impl Transport for MockTransport {
    fn open(&self, _: ConnectRequest) -> Session {
        let (session, driver) = nocterm_session::channel(None);
        self.drivers.lock().unwrap().push(Arc::new(driver));
        session
    }
}

fn spec(host: &str) -> SessionSpec {
    SessionSpec {
        profile: None,
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
    if vault_ready {
        // The vault worker is a real thread; let its completions wake the test scheduler.
        cx.executor().allow_parking();
    }
    fixture_with_theme_option(cx, vault_ready, None, false)
}

fn fixture_with_themes(
    cx: &mut TestAppContext,
    themes: Arc<nocterm_themes::test_support::InMemoryThemes>,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<TerminalView>,
    Arc<MockTransport>,
    nocterm_store::SettingsHandle,
) {
    let (window, workspace, terminal, transport, _, policy) =
        fixture_with_settings(cx, false, Some(themes), false, None);
    (window, workspace, terminal, transport, policy)
}

fn fixture_with_theme_option(
    cx: &mut TestAppContext,
    vault_ready: bool,
    themes: Option<Arc<nocterm_themes::test_support::InMemoryThemes>>,
    with_local: bool,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<TerminalView>,
    Arc<MockTransport>,
) {
    let (window, workspace, terminal, transport, _) =
        fixture_with_keymap(cx, vault_ready, themes, with_local);
    (window, workspace, terminal, transport)
}

fn fixture_with_keymap(
    cx: &mut TestAppContext,
    vault_ready: bool,
    themes: Option<Arc<nocterm_themes::test_support::InMemoryThemes>>,
    with_local: bool,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<TerminalView>,
    Arc<MockTransport>,
    nocterm_keymap::KeymapHandle,
) {
    let (window, workspace, terminal, transport, keymap, _) =
        fixture_with_settings(cx, vault_ready, themes, with_local, None);
    (window, workspace, terminal, transport, keymap)
}

fn fixture_with_settings(
    cx: &mut TestAppContext,
    vault_ready: bool,
    themes: Option<Arc<nocterm_themes::test_support::InMemoryThemes>>,
    with_local: bool,
    settings_override: Option<nocterm_store::SettingsConfig>,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<TerminalView>,
    Arc<MockTransport>,
    nocterm_keymap::KeymapHandle,
    nocterm_store::SettingsHandle,
) {
    let transport = Arc::new(MockTransport::default());
    let (window, workspace, terminal, keymap, policy) = cx.update(|cx| {
        let mut settings = nocterm_settings::SettingsDocument::default();
        let directory = tempfile::tempdir().unwrap();
        settings.update::<nocterm_session::LocalShellSettings>(|section| {
            section.cwd = Some(directory.path().to_string_lossy().into_owned())
        });
        let themes = themes.map(|themes| {
            use nocterm_themes::ThemeLibrary as _;
            crate::bootstrap::Themes {
                catalog: themes.scan(),
                library: themes.clone(),
                registry: themes,
            }
        });
        let booted = boot_with_ports(
            directory,
            settings_override.unwrap_or_else(|| nocterm_store::SettingsConfig::in_memory(settings)),
            (
                transport.clone(),
                if with_local {
                    let local = transport.clone();
                    nocterm_terminal::LocalSessions::new(
                        Arc::new(move |_| local.clone()),
                        "/bin/bash".into(),
                    )
                } else {
                    nocterm_terminal::LocalSessions::disabled("/bin/bash".into())
                },
            ),
            vault_ready,
            None,
            themes,
            cx,
        );
        if let Some(installed) = booted.vault() {
            let service = installed.service();
            // Creates the vault with a master password and leaves it locked.
            cx.set_global(FixtureVault(Box::new(move || {
                futures::executor::block_on(
                    service.create(nocterm_session::Secret::new("long master password")),
                )
                .unwrap();
                service.lock();
            })));
        }
        let (window, workspace) = open_workspace(&booted, cx);
        let terminal = window
            .update(cx, |_, window, cx| {
                workspace.update(cx, |workspace, cx| {
                    workspace.open_session(spec("first.test"), window, cx)
                });
                workspace.read(cx).find_item::<TerminalView>().unwrap()
            })
            .unwrap();
        (
            window,
            workspace,
            terminal,
            booted.keymap(),
            booted.settings_deps().settings().clone(),
        )
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
    (window, workspace, terminal, transport, keymap, policy)
}

/// Opens a window with the application's workspace.
fn open_workspace(
    booted: &crate::bootstrap::Booted,
    cx: &mut gpui_kit::App,
) -> (AnyWindowHandle, Entity<Workspace>) {
    gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
        cx.new(|cx| {
            let mut workspace = Workspace::new(booted.workspace_deps(), window, cx);
            crate::bootstrap::build_workspace(&mut workspace, booted, window, cx);
            workspace
        })
    })
    .unwrap()
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

struct FixtureVault(Box<dyn Fn()>);
impl gpui_kit::Global for FixtureVault {}

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
#[expect(clippy::too_many_lines, reason = "predates the limit")]
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
fn transfers_action_uses_installed_queue_before_explorer_opens(cx: &mut TestAppContext) {
    let (handle, workspace, _, _) = fixture(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("local-browser").is_none());
        window.dispatch_action(Box::new(nocterm_files::ShowTransfers), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            workspace
                .read(cx)
                .active_item()
                .unwrap()
                .tab_title(cx)
                .as_ref(),
            "Transfers"
        );
        window.dispatch_action(Box::new(nocterm_files::ShowTransfers), cx);
    })
    .unwrap();
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace
                .items()
                .filter(|item| item.tab_title(cx).as_ref() == "Transfers")
                .count(),
            1
        );
    });
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

#[gpui_kit::test]
fn ai_panel_and_settings_actions_are_available_and_master_switch_hides_toggle(
    cx: &mut TestAppContext,
) {
    let (window, workspace, _, _, _, policy) = fixture_with_settings(cx, false, None, false, None);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).right_panel_is_available());
        window.dispatch_action(Box::new(nocterm_workspace::ToggleRightPanel), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        assert!(workspace.read(cx).right_panel_is_open());
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_workspace::OpenAiSettings), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        let settings = workspace
            .read(cx)
            .find_item::<nocterm_settings_ui::SettingsView>()
            .unwrap();
        assert_eq!(settings.read(cx).selected_page_id(), "ai");
        policy
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.enabled = false)
            .detach();
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, _, cx| {
        assert!(!workspace.read(cx).right_panel_is_available());
        assert!(!workspace.read(cx).right_panel_is_open());
    })
    .unwrap();
}

#[gpui_kit::test]
fn terminal_context_serialization_excludes_source_credential_launch_and_proxy_markers(
    cx: &mut TestAppContext,
) {
    use nocterm_ai::context::{ConnectionDescriptor, TerminalDescriptor, context_block};
    let (window, workspace, _, _) = fixture(cx);
    let markers = [
        "PRIVATE_KEY_PATH_MARKER",
        "LAUNCH_ENV_SECRET_MARKER",
        "PROXY_HOST_PRIVATE_MARKER",
        "c0ffee00c0ffee00c0ffee00c0ffee00",
    ];
    cx.update_window(window, |_, window, cx| {
        let mut source = spec("safe.example");
        source.auth = nocterm_session::Auth::Key {
            path: std::path::PathBuf::from(markers[0]),
        };
        source.launch = Some(nocterm_session::ShellLaunch {
            env: std::collections::BTreeMap::from([("PRIVATE_VAR".into(), markers[1].into())]),
            ..Default::default()
        });
        source.credential = Some(markers[3].parse().unwrap());
        source.options.proxy = Some(nocterm_session::ProxyConfig::HttpConnect {
            host: markers[2].into(),
            port: 8080,
        });
        workspace.update(cx, |workspace, cx| {
            workspace.open_session(source, window, cx)
        });
        let descriptors = workspace
            .read(cx)
            .terminals(cx)
            .into_iter()
            .map(|entry| {
                let info = entry.capabilities.state().info(cx).unwrap();
                TerminalDescriptor {
                    id: format!("{:?}", entry.item),
                    title: entry.title.to_string(),
                    local: info.local,
                    cwd: info.cwd.map(|p| p.to_string_lossy().into_owned()),
                    status: format!("{:?}", info.status),
                    connection: info.target.map(|target| ConnectionDescriptor {
                        id: String::new(),
                        name: info.title.to_string(),
                        group: None,
                        description: String::new(),
                        host: target.host,
                        port: target.port,
                        user: target.user,
                    }),
                }
            })
            .collect::<Vec<_>>();
        let context = context_block(&descriptors, &[]);
        let list_response = nocterm_ai::mcp::tool_result(
            serde_json::json!(1),
            Ok(serde_json::json!({"context":context})),
        )
        .to_string();
        assert!(list_response.contains("safe.example"));
        for marker in markers {
            assert!(
                !list_response.contains(marker),
                "Source field leaked: {marker}"
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn agent_header_lines_up_with_the_tab_strip(cx: &mut TestAppContext) {
    let (window, _, terminal, _) = fixture(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(nocterm_workspace::ToggleRightPanel), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let header = window.find("agent-header").bounds();
        let tab = window
            .find(("tab-title", terminal.entity_id().as_u64()))
            .bounds();
        assert_eq!(header.size.height, gpui_kit::px(32.));
        assert!(
            (header.center().y - tab.center().y).abs() <= gpui_kit::px(1.),
            "{header:?} {tab:?}"
        );
    })
    .unwrap();
}

mod settings_pages;
