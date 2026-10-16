//! nocterm: the application.
//!
//! This is the composition root. It reads the user's files, installs the
//! globals every crate expects, and plugs the features into one workspace
//! window. It is the only crate that names every other one; nothing else
//! does any wiring.

// Release builds are GUI applications without a console window on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent_auth;
mod app_menus;
mod application;
mod bootstrap;
mod connection_setup;
mod keymap;
mod startup;

#[cfg(test)]
mod gui_tests;

use std::io;

use gpui_kit::{
    App, AppContext as _, Bounds, Focusable as _, WindowBounds, WindowOptions, component::TitleBar,
    px, size,
};
use nocterm_ui::SettingsStore;
use nocterm_workspace::{
    ChangeColorScheme, ChangeTheme, DefaultSessionSettings, OpenAiSettings, OpenKeymap,
    OpenSSHSettings, OpenVault, Workspace,
};
use tracing_subscriber::EnvFilter;

/// Used by Linux desktops to match the window to its `.desktop` entry.
const APP_ID: &str = "dev.nocterm.Nocterm";
/// The subcommand agents run to reach a chat's terminal tools.
const AGENT_BRIDGE: &str = "agent-bridge";

fn main() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "agent-host")
    {
        return nocterm_acp::run_agent_host(std::env::args_os().skip(2))
            .map_err(anyhow::Error::from);
    }
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == AGENT_BRIDGE)
    {
        return nocterm_acp::run_relay_from_environment().map_err(anyhow::Error::from);
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(io::stderr)
        .init();

    let prepared = startup::PreparedApp::load()?;
    gpui_kit::application()
        .with_assets(nocterm_ui::Assets)
        .run(move |cx| prepared.install_when_ready(cx));
    Ok(())
}

fn open_main_window(cx: &mut App, booted: &bootstrap::Booted) -> anyhow::Result<()> {
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
        let themes = booted.themes.clone();
        window
            .observe_window_appearance(move |_, cx| themes.apply(cx))
            .detach();

        let workspace = cx.new(|cx| {
            let mut workspace = Workspace::new(booted.workspace_deps(), window, cx);
            bootstrap::build_workspace(&mut workspace, booted, window, cx);
            workspace
        });
        window.focus(&workspace.focus_handle(cx), cx);
        workspace
    })?;
    Ok(())
}

fn register_settings(
    workspace: &mut Workspace,
    vault: Option<&nocterm_vault_ui::Installed>,
    themes: nocterm_ui::ThemeUi,
    deps: nocterm_settings_ui::Installed,
    keymap: nocterm_keymap::KeymapHandle,
    files: &nocterm_files::Installed,
    monitor: &nocterm_monitor_ui::Installed,
    terminal_pages: [nocterm_ui::SettingsPageSpec; 3],
    ai_page: nocterm_ui::SettingsPageSpec,
) {
    let mut pages = vec![
        nocterm_keymap_ui::settings_page(keymap),
        nocterm_monitor_ui::settings_page(monitor),
        nocterm_files::settings_page(files),
    ];
    if let Some(vault) = vault {
        pages.push(nocterm_vault_ui::settings_page(vault));
    }
    register_appearance_commands(workspace, themes.clone(), deps.settings().clone());
    let pages = settings_owner_pages(themes, terminal_pages, ai_page, pages);
    nocterm_settings_ui::register(workspace, pages.clone(), deps.clone());
    /// Opens Settings at page `id` when `A` runs.
    fn open_on<A: gpui_kit::Action>(
        workspace: &mut Workspace,
        id: &'static str,
        pages: &nocterm_settings_ui::OwnerPages,
        deps: nocterm_settings_ui::Installed,
    ) {
        let pages = pages.clone();
        workspace.register_action(move |_, _: &A, window, cx| {
            let workspace = cx.entity().downgrade();
            let pages = pages.clone();
            let deps = deps.clone();
            window.defer(cx, move |window, cx| {
                let _ = workspace.update(cx, |workspace, cx| {
                    nocterm_settings_ui::open_page(workspace, id, &pages, deps, window, cx);
                });
            });
        });
    }
    open_on::<DefaultSessionSettings>(workspace, "ssh", &pages, deps.clone());
    open_on::<OpenAiSettings>(workspace, "ai", &pages, deps.clone());
    open_on::<OpenSSHSettings>(workspace, "ssh", &pages, deps.clone());
    open_on::<OpenKeymap>(workspace, "keymap", &pages, deps.clone());
    open_on::<nocterm_monitor_ui::OpenMonitorSettings>(workspace, "monitor", &pages, deps.clone());
    open_on::<nocterm_files::OpenExplorerSettings>(workspace, "explorer", &pages, deps.clone());
    if vault.is_some() {
        open_on::<OpenVault>(workspace, "vault", &pages, deps.clone());
    }
}

/// Logs the sections of the settings file that could not be read. Every
/// section is registered once the window's features are.
fn report_settings_errors(cx: &App) {
    for error in cx.global::<SettingsStore>().section_errors(cx) {
        tracing::error!(%error, "a settings section is invalid; using its defaults");
    }
}

fn settings_owner_pages(
    themes: nocterm_ui::ThemeUi,
    terminal: [nocterm_ui::SettingsPageSpec; 3],
    ai: nocterm_ui::SettingsPageSpec,
    guests: Vec<nocterm_ui::SettingsPageSpec>,
) -> nocterm_settings_ui::OwnerPages {
    let [terminal, local, ssh] = terminal;
    nocterm_settings_ui::OwnerPages::new(
        [
            nocterm_ui::appearance_settings_page(themes),
            terminal,
            local,
            ssh,
            ai,
        ],
        guests,
    )
}
fn register_appearance_commands(
    workspace: &mut Workspace,
    themes: nocterm_ui::ThemeUi,
    settings: nocterm_store::SettingsHandle,
) {
    let picker = themes.clone();
    let theme_settings = settings.clone();
    workspace.register_action(move |_, _: &ChangeTheme, window, cx| {
        let themes = picker.clone();
        let settings = theme_settings.clone();
        window.defer(cx, move |window, cx| {
            nocterm_ui::open_theme_picker(&themes, &settings, window, cx)
        });
    });
    workspace.register_action(move |_, _: &ChangeColorScheme, window, cx| {
        let settings = settings.clone();
        window.defer(cx, move |window, cx| {
            nocterm_ui::open_scheme_picker(&settings, window, cx)
        });
    });
}
