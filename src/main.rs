//! nocterm: the application.
//!
//! This is the composition root. It reads the user's files, installs the
//! retained service dependencies, validates settings catalogs and plugs the features
//! into each workspace window. It is the only crate that names every other one; nothing else
//! does any wiring.

// Release builds are GUI applications without a console window on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod agent_auth;
mod app_menus;
mod application;
mod bootstrap;
mod connection_setup;
mod keymap;
#[cfg(feature = "user-plugins")]
mod plugin_command;
mod project_setup;
mod settings_pages;
mod shell_setup;
mod startup;

#[cfg(test)]
mod agent_history_tests;
#[cfg(test)]
mod gui_tests;

use std::io;

use gpui_kit::{
    App, AppContext as _, Bounds, Focusable as _, WindowBounds, WindowOptions, component::TitleBar,
    px, size,
};

use nocterm_workspace::Workspace;
use tracing_subscriber::EnvFilter;

/// Used by Linux desktops to match the window to its `.desktop` entry.
const APP_ID: &str = "dev.nocterm.Nocterm";
/// The subcommand agents run to reach a chat's terminal tools.
const AGENT_BRIDGE: &str = "agent-bridge";

fn main() -> anyhow::Result<()> {
    #[cfg(feature = "user-plugins")]
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "plugin")
    {
        std::process::exit(plugin_command::run(std::env::args_os().skip(2)));
    }
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
        .with_assets(nocterm_ui::Assets::with_marks(
            nocterm_agent_providers::catalog()
                .marks()
                .map(|mark| (mark.asset(), mark.svg()))
                .chain(nocterm_projects::marks()),
        ))
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
        let themes = booted.themes().clone();
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

/// Logs the sections of the settings file that could not be read. Every
/// section is registered once the window's features are.
fn report_settings_errors(settings: &nocterm_store::SettingsHandle, cx: &App) {
    for error in settings.section_errors(cx) {
        tracing::error!(%error, "a settings section is invalid; using its defaults");
    }
}
