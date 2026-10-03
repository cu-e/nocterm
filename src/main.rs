//! nocterm: the application.
//!
//! This is the composition root. It reads the user's files, installs the
//! globals every crate expects, and plugs the features into one workspace
//! window. It is the only crate that names every other one; nothing else
//! does any wiring.

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
use nocterm_ui::SettingsStore;
use nocterm_workspace::{Quit, Workspace};
use tracing_subscriber::EnvFilter;

/// Used by Linux desktops to match the window to its `.desktop` entry.
const APP_ID: &str = "dev.nocterm.Nocterm";

fn main() -> anyhow::Result<()> {
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
            nocterm_terminal::init(Arc::new(transport), cx);
            nocterm_terminal::init_recording(paths.state_dir().join("logs"), cx);
            nocterm_terminal::init_local(
                Arc::new(|launch| Arc::new(nocterm_local::LocalTransport(launch))),
                cx,
            );
            nocterm_connections::init(Some(&paths), cx);
            let vault_ready = match nocterm_vault_ui::init(paths.config_dir().join("vault.bin"), cx)
            {
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
            keymap::load(cx);

            cx.on_action(|_: &Quit, cx| cx.quit());
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
        // "System" appearance follows the desktop as it changes.
        window
            .observe_window_appearance(|_, cx| nocterm_ui::apply_theme(cx))
            .detach();

        let workspace = cx.new(|cx| {
            let mut workspace = Workspace::new(window, cx);
            workspace.set_session_opener(nocterm_terminal::open_session);
            workspace.set_local_terminal_opener(nocterm_terminal::open_local);
            nocterm_connections::register(&mut workspace, window, cx);
            nocterm_settings_ui::register(&mut workspace);
            nocterm_files::register(&mut workspace, cx);
            if vault_ready {
                nocterm_vault_ui::register(&mut workspace);
            }
            workspace
        });
        window.focus(&workspace.focus_handle(cx), cx);
        workspace
    })?;
    Ok(())
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
