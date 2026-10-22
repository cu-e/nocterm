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
mod chat_history;
mod keymap;
mod menu_tabs;
mod program_opening;
mod project_workspaces;
mod secret_prompts;
mod shortcuts;
mod snippets;
mod terminal_events;
mod titlebar;
mod vault;

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
        profile: None,
        options: Default::default(),
        title: host.to_owned().into(),
        target: nocterm_session::Target::new("test", host, 22),
        auth: nocterm_session::Auth::Auto,
        launch: None,
        credential: None,
        jumps: Vec::new(),
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
                workspace.read(cx).find_item::<TerminalView>(cx).unwrap()
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

/// Installs the application's globals as the application does, with the
/// state in `directory` and nothing persisted.
fn boot(
    directory: tempfile::TempDir,
    settings: nocterm_settings::SettingsDocument,
    transport: Arc<MockTransport>,
    vault: bool,
    cx: &mut gpui_kit::App,
) -> crate::bootstrap::Booted {
    boot_with_files(directory, settings, transport, vault, None, cx)
}

fn boot_with_files(
    directory: tempfile::TempDir,
    settings: nocterm_settings::SettingsDocument,
    transport: Arc<MockTransport>,
    vault: bool,
    files: Option<nocterm_files::Deps>,
    cx: &mut gpui_kit::App,
) -> crate::bootstrap::Booted {
    boot_with_ports(
        directory,
        nocterm_store::SettingsConfig::in_memory(settings),
        (
            transport,
            nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
        ),
        vault,
        files,
        None,
        cx,
    )
}
fn boot_with_ports(
    directory: tempfile::TempDir,
    settings: nocterm_store::SettingsConfig,
    terminal: (Arc<MockTransport>, nocterm_terminal::LocalSessions),
    vault: bool,
    files: Option<nocterm_files::Deps>,
    themes: Option<crate::bootstrap::Themes>,
    cx: &mut gpui_kit::App,
) -> crate::bootstrap::Booted {
    boot_with_ports_and_probe(
        directory,
        settings,
        terminal,
        vault,
        files,
        themes,
        Arc::new(nocterm_ai::testing::ScriptedSandboxProbe::new(
            nocterm_ai::sandbox::Availability::Unsupported,
        )),
        cx,
    )
}
fn boot_with_ports_and_probe(
    directory: tempfile::TempDir,
    settings: nocterm_store::SettingsConfig,
    terminal: (Arc<MockTransport>, nocterm_terminal::LocalSessions),
    vault: bool,
    files: Option<nocterm_files::Deps>,
    themes: Option<crate::bootstrap::Themes>,
    probe: Arc<dyn nocterm_ai::sandbox::SandboxProbe>,
    cx: &mut gpui_kit::App,
) -> crate::bootstrap::Booted {
    let (transport, local) = terminal;
    let local_host = local.is_available().then(|| {
        Arc::new(nocterm_session::testing::FakeExec::default())
            as Arc<dyn nocterm_session::HostExec>
    });
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    let vault = vault.then(|| crate::bootstrap::VaultSetup {
        file: directory.path().join("vault.bin"),
        device_unlock: false,
    });
    let files = files.unwrap_or_else(|| fixture_files(directory.path()));
    cx.set_global(FixtureDirectory {
        _directory: directory,
    });
    let booted = crate::bootstrap::bootstrap(
        crate::bootstrap::Services {
            images: {
                let codec = Arc::new(nocterm_ai::testing::ScriptedImageCodec::default());
                nocterm_agent::ImageDeps::new(codec.clone(), codec)
            },
            tokens: nocterm_ui::DesignTokens::builtin(),
            settings,
            persist: false,
            connections: crate::connection_setup::memory(),
            projects: crate::project_setup::PreparedProjectSetup::memory(),
            hosts: crate::bootstrap::HostServices {
                transport,
                local,
                local_host,
            },
            terminal_auth_factory: None,
            files,
            themes,
            sandbox_probe: probe,
            agent: nocterm_agent::AgentServices {
                catalog: nocterm_agent_providers::catalog(),
                metadata: Arc::new(nocterm_ai::testing::ScriptedChatMetadata::default()),
                connector: Arc::new(nocterm_acp::AcpConnector::unmanaged()),
                bridge: Arc::new(nocterm_acp::BridgeServer::new(
                    paths.clone(),
                    nocterm_acp::RelayCommand {
                        program: std::env::current_exe().expect("test executable"),
                        args: vec![crate::AGENT_BRIDGE.into()],
                    },
                )),
                storage: futures::executor::block_on(nocterm_agent_runtime::AgentStorage::prepare(
                    Arc::new(nocterm_agent_storage::FileAgentStateRepository::new(
                        paths.state_dir().join("agents.toml"),
                    )),
                    Arc::new(nocterm_agent_storage::FileChatRepository::new(
                        paths.state_dir().join("agent-chats"),
                        Arc::new(nocterm_localfs::LocalFilesystem::new().unwrap()),
                    )),
                )),
                workdir_resolver: Arc::new(nocterm_acp::NativeWorkdirResolver::default()),
                usage_source: Arc::new(nocterm_ai::testing::ScriptedUsageSource::default()),
                workdir: paths.state_dir().join("agent-workspace"),
                terminal_auth: None,
                private_dirs: Vec::new(),
                shared_dirs: Vec::new(),
            },
            paths,
            vault,
        },
        cx,
    )
    .expect("valid fixture settings catalogs");
    cx.set_reduce_motion(true);
    booted
}

fn fixture_files(directory: &std::path::Path) -> nocterm_files::Deps {
    let local = Arc::new(nocterm_session::testing::InMemoryFs::default());
    seed_native_ancestors(&local, directory);
    let transfers = nocterm_transfer_runtime::TransferRuntime::start(
        nocterm_transfer_runtime::TransferRuntimeDeps::new(local.clone(), Default::default()),
    )
    .unwrap();
    nocterm_files::Deps::new(
        local.clone(),
        local,
        transfers,
        Arc::new(nocterm_session::testing::FakeExec::default()),
    )
}
fn seed_native_ancestors(fs: &nocterm_session::testing::InMemoryFs, directory: &std::path::Path) {
    for ancestor in directory.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if ancestor.parent().is_some() {
            fs.seed_native_directory(ancestor).unwrap();
        }
    }
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
                .active_item(cx)
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
                .items(cx)
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
            .find_item::<nocterm_settings_ui::SettingsView>(cx)
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
