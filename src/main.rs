//! nocterm: the application.
//!
//! This is the composition root. It reads the user's files, installs the
//! globals every crate expects, and plugs the features into one workspace
//! window. It is the only crate that names every other one; nothing else
//! does any wiring.

mod agent_auth;
mod agent_bridge;
mod app_menus;
mod application;
mod keymap;

#[cfg(test)]
mod gui_tests;

use std::{fs, io, sync::Arc};

use anyhow::Context as _;
use gpui_kit::{
    App, AppContext as _, Bounds, Focusable as _, WindowBounds, WindowOptions, component::TitleBar,
    px, size,
};
use nocterm_core::Paths;
use nocterm_design::DesignTokens;
use nocterm_settings::{Settings, SettingsFile};
use nocterm_ssh::{SshConfig, SshTransport};
use nocterm_ui::{ActiveSettings as _, SettingsStore};
use nocterm_workspace::{
    DefaultSessionSettings, OpenAiSettings, OpenKeymap, OpenSSHSettings, OpenVault, Workspace,
};
use tracing_subscriber::EnvFilter;

/// Used by Linux desktops to match the window to its `.desktop` entry.
const APP_ID: &str = "dev.nocterm.Nocterm";

fn main() -> anyhow::Result<()> {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "agent-bridge")
    {
        return agent_bridge::run_from_environment().map_err(anyhow::Error::msg);
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(io::stderr)
        .init();

    let paths = Paths::discover()?;
    let tokens = load_tokens(&paths);
    let settings = load_settings(&paths);
    let transport = SshTransport::new(ssh_config(&paths)).context("starting the SSH runtime")?;

    gpui_kit::application()
        .with_assets(nocterm_ui::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            nocterm_ui::init(tokens, settings, cx);
            nocterm_ui::LayoutMemory::init(Some(paths.state_dir().join("layout.json")), cx);
            nocterm_terminal::init(Arc::new(transport), cx);
            nocterm_terminal::init_recording(paths.state_dir().join("logs"), cx);
            nocterm_terminal::init_local(
                Arc::new(|launch| Arc::new(nocterm_local::LocalTransport(launch))),
                cx,
            );
            nocterm_connections::init(Some(&paths), cx);
            nocterm_agent::init(
                nocterm_agent::AgentServices {
                    connector: Arc::new(nocterm_acp::AcpConnector),
                    terminal_auth: Some(Arc::new(agent_auth::open)),
                    bridge: Arc::new(nocterm_acp::BridgeServer::new(paths.clone())),
                    state_file: paths.state_dir().join("agents.toml"),
                    chats_dir: paths.state_dir().join("agent-chats"),
                    codex_home: nocterm_ai::usage::codex_home(),
                    workdir: paths.state_dir().join("agent-workspace"),
                    private_dirs: vec![paths.config_dir().to_owned(), paths.state_dir().to_owned()],
                    shared_dirs: vec![paths.effective_runtime_dir()],
                },
                cx,
            );
            let vault_ready = match nocterm_vault_ui::init_with_device_unlock(
                paths.config_dir().join("vault.bin"),
                Some(nocterm_device_unlock::provider()),
                cx,
            ) {
                Ok(vault) => {
                    nocterm_terminal::init_credentials(
                        vault,
                        |spec, id, cx| {
                            nocterm_connections::Connections::global(cx).update(
                                cx,
                                |connections, cx| {
                                    connections.associate_credential(
                                        &spec.target,
                                        &spec.auth,
                                        id,
                                        cx,
                                    )
                                },
                            )
                        },
                        cx,
                    );
                    true
                }
                Err(error) => {
                    tracing::error!(%error, "could not initialize the credential vault");
                    false
                }
            };
            keymap::load(Some(paths.config_dir().join("keymap.toml")), cx);

            application::register(paths.clone(), vault_ready, cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            if let Err(error) = open_main_window(cx, vault_ready) {
                tracing::error!(%error, "could not open the window");
                cx.quit();
            }
            cx.activate(true);
        });
    Ok(())
}

fn open_main_window(cx: &mut App, vault_ready: bool) -> anyhow::Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1200.), px(760.)),
            cx,
        ))),
        window_min_size: Some(size(px(640.), px(400.))),
        app_id: Some(APP_ID.to_owned()),
        ..TitleBar::window_options()
    };

    gpui_kit::open_window(options, cx, |window, cx| {
        window.set_window_title("Nocterm");
        // "System" appearance follows the desktop as it changes.
        window
            .observe_window_appearance(|_, cx| nocterm_ui::apply_theme(cx))
            .detach();

        let workspace = cx.new(|cx| {
            let mut workspace = Workspace::new(window, cx);
            workspace.set_session_opener(nocterm_terminal::open_session);
            workspace.set_background_session_opener(nocterm_terminal::open_background_session);
            workspace.set_local_terminal_opener(nocterm_terminal::open_local);
            nocterm_connections::register(&mut workspace, window, cx);
            register_settings(&mut workspace, vault_ready);
            nocterm_files::register(&mut workspace, window, cx);
            nocterm_agent::register(&mut workspace, window, cx);
            workspace.set_menu_builder(app_menus::build, window, cx);
            if vault_ready && cx.settings().vault.prompt_on_startup {
                let pages = vec![nocterm_vault_ui::settings_page()];
                nocterm_settings_ui::open_page(&mut workspace, "vault", &pages, window, cx);
            }
            workspace
        });
        window.focus(&workspace.focus_handle(cx), cx);
        workspace
    })?;
    Ok(())
}

fn register_settings(workspace: &mut Workspace, vault_ready: bool) {
    let mut pages = vec![nocterm_keymap_ui::settings_page()];
    if vault_ready {
        pages.push(nocterm_vault_ui::settings_page());
    }
    nocterm_settings_ui::register_with_pages(workspace, pages.clone());
    /// Opens Settings at page `id` when `A` runs.
    fn open_on<A: gpui_kit::Action>(
        workspace: &mut Workspace,
        id: &'static str,
        pages: &[nocterm_workspace::SettingsPageSpec],
    ) {
        let pages = pages.to_vec();
        workspace.register_action(move |_, _: &A, window, cx| {
            let workspace = cx.entity().downgrade();
            let pages = pages.clone();
            window.defer(cx, move |window, cx| {
                let _ = workspace.update(cx, |workspace, cx| {
                    nocterm_settings_ui::open_page(workspace, id, &pages, window, cx);
                });
            });
        });
    }
    open_on::<DefaultSessionSettings>(workspace, "ssh", &pages);
    open_on::<OpenAiSettings>(workspace, "ai", &pages);
    open_on::<OpenSSHSettings>(workspace, "ssh", &pages);
    open_on::<OpenKeymap>(workspace, "keymap", &pages);
    if vault_ready {
        open_on::<OpenVault>(workspace, "vault", &pages);
    }
}

/// The built-in design tokens with the user's `theme.toml` over them. A
/// broken theme is reported and ignored, so the window still opens.
fn load_tokens(paths: &Paths) -> DesignTokens {
    let file = paths.theme_file();
    let overrides = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return DesignTokens::builtin(),
        Err(error) => {
            tracing::error!(file = %file.display(), %error, "could not read the theme");
            return DesignTokens::builtin();
        }
    };
    DesignTokens::with_overrides(&overrides).unwrap_or_else(|error| {
        tracing::error!(file = %file.display(), %error, "the theme is invalid; using the default");
        DesignTokens::builtin()
    })
}

/// The user's settings. Broken settings are reported and replaced by the
/// defaults for this run only: the file is left alone so nothing in it is
/// lost.
fn load_settings(paths: &Paths) -> SettingsStore {
    let file = SettingsFile::new(paths.settings_file());
    match file.load() {
        Ok(settings) => SettingsStore::new(settings, file),
        Err(error) => {
            tracing::error!(%error, "could not read the settings; changes will not be saved");
            SettingsStore::in_memory(Settings::default())
        }
    }
}

fn ssh_config(paths: &Paths) -> SshConfig {
    let openssh = paths.openssh_dir();
    SshConfig {
        known_hosts: paths.known_hosts_file(),
        read_only_known_hosts: openssh.iter().map(|dir| dir.join("known_hosts")).collect(),
        identity_dir: openssh,
        use_agent: true,
    }
}
