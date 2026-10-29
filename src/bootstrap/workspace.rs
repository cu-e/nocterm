//! Plugs the features into the workspace of one window.

use std::rc::Rc;

use gpui_kit::{Context, Window};
use nocterm_hubs_model::Environment;
use nocterm_workspace::Workspace;

use super::{Booted, report_registration};
use crate::window_sources::WindowSources;

/// Plugs every feature into a new window's workspace.
///
/// What depends on the window's source (connections, terminal, vault and their settings
/// pages) comes from `sources`; the rest is shared by every window and comes from `booted`.
pub(crate) fn build_workspace(
    workspace: &mut Workspace,
    booted: &Booted,
    sources: &WindowSources,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let vault = sources.vault().map(Rc::as_ref);
    if let (Some(vault), Environment::Hub(_)) = (vault, sources.environment()) {
        // The vault of a hub answers `UnlockVault` in this window only.
        nocterm_vault_ui::register_scoped(vault, workspace);
    }
    nocterm_terminal::register(sources.terminal(), workspace);
    if let Some(connections) = sources.connections() {
        report_registration(
            "connections",
            nocterm_connections::register(connections, workspace, window, cx),
        );
        workspace.set_chat_connection_opener(crate::project_setup::chat_connection_opener(
            sources.terminal(),
            connections,
        ));
    }
    sources
        .settings()
        .register(workspace, booted.themes().clone());
    report_registration(
        "files",
        nocterm_files::register(&booted.files, workspace, window, cx),
    );
    if let Some(snippets) = &booted.snippets {
        report_registration(
            "snippets",
            nocterm_snippets_ui::register(workspace, snippets, window, cx),
        );
    }
    report_registration(
        "containers",
        nocterm_containers_ui::register(booted.local_host.clone(), workspace, window, cx),
    );
    // Workspaces, like containers and files, work over the connection the window opened,
    // whichever source that connection came from. The terminals they open and the profiles
    // they resolve are those of this window's source.
    if let Some(projects) = booted.projects_of(sources) {
        report_registration(
            "projects",
            nocterm_projects::register(workspace, &projects, window, cx),
        );
    }
    let user = booted.user_plugins.window(&booted.files);
    report_registration(
        "monitor",
        user.register_monitor(&booted.monitor, workspace, window, cx),
    );
    let agent = sources.agent_installation(&booted.agent);
    report_registration(
        "agent",
        nocterm_agent::register(workspace, agent, window, cx),
    );
    workspace.set_menu_builder(crate::app_menus::build, window, cx);
    if let Some(hubs) = booted.hubs() {
        nocterm_hubs_ui::register(
            hubs,
            sources.environment().clone(),
            crate::windows::AppEnvironmentOpener::new(booted.clone()),
            workspace,
            window,
            cx,
        );
    }
    user.attach(workspace, window, cx);
    if let Some(vault) = vault
        && sources.environment().is_local()
        && vault
            .settings()
            .setting::<nocterm_vault_ui::VaultSettings>(cx)
            .prompt_on_startup
        && let Some(pages) = sources.settings().startup()
    {
        nocterm_settings_ui::open_page(
            workspace,
            "vault",
            pages,
            sources.settings().installed(),
            window,
            cx,
        );
    }
}
