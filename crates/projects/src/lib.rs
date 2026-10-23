//! Workspace creation and presentation across explicit installed providers.
//!
//! Owns: saved workspace library, three-mode native creation controls, shared
//! directory-picker consumption, image search/version draft owners, Workspaces settings controls, observed environment
//! settings with explicit tool changes, explicit Keep/Stop/Delete removal controls,
//! and one outer Item with inner terminal tabs.
//! Does not own: sessions, transport I/O, catalog persistence or chat documents.
//! Ports used: ProjectsHandle/WorkspaceSetup, captured preparation/ImageSearch, DirectoryRead,
//! bound ProjectTerminalSession/ProjectTerminalResolve, WorkspaceEnvironment
//! and WorkspaceContainerActions. Ports provided:
//! retained Installed registration and ProjectView.
//! Workspace remains for outer Item admission, Panel/actions, focused-source capture,
//! original General Item enumeration and connection-directory projections (ADR0079).
//! Project terminal contracts/refusals come directly from workspace-ports; containers supplies DTOs.
//! Settings section: `[workspaces]`. Globals: none.
//! Invariants: original source capabilities, checked target and ProjectId remain
//! stable; an injected local source permits creation without a terminal, while
//! selected remote contexts refuse rather than redirect locally. Unknown providers
//! preserve metadata/history; delayed recovery checks
//! the original binding/claim and never steals changed focus.

mod context;
mod editor;
mod environment;
mod error;
mod installation;
mod languages;
mod location;
mod opening;
mod panel;
mod progress;
mod removal;
mod settings_page;
pub use settings_page::settings_page;
mod view;

pub use error::ProjectOpenError;
pub use installation::Installed;
pub use panel::ProjectsPanel;
pub use view::ProjectView;

use gpui_kit::{Context, Window};
use nocterm_workspace::Workspace;

gpui_kit::actions!(
    projects,
    [
        /// Show saved workspaces for the current connection.
        ToggleProjects,
        /// Create a workspace on the current server or this computer.
        NewProject,
    ]
);

pub fn init(
    model: nocterm_project_service::ProjectsHandle,
    terminals: std::rc::Rc<dyn nocterm_workspace_ports::ProjectTerminalOpen>,
    settings: nocterm_store::SettingsHandle,
    cx: &mut gpui_kit::App,
) -> Installed {
    Installed::new(model, terminals, settings, cx)
}

/// Registration order puts this footer switch immediately after Containers.
pub fn register(
    workspace: &mut Workspace,
    installed: &Installed,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    use gpui_kit::AppContext as _;
    let owner = cx.entity().downgrade();
    let installed = installed.clone();
    let panel = cx.new(|cx| ProjectsPanel::new(installed.clone(), owner, window, cx));
    workspace.add_panel(panel, cx);
    workspace.register_action(|workspace, _: &ToggleProjects, window, cx| {
        workspace.toggle_panel_of::<ProjectsPanel>(window, cx);
    });
    workspace.register_action(move |workspace, _: &NewProject, window, cx| {
        match context::capture_current(workspace, &installed, window, cx) {
            Ok(source) => {
                editor::open(installed.clone(), source, window, cx);
            }
            Err(error) => nocterm_ui::notice::error(
                window,
                cx,
                "project-create",
                "Select a connected server",
                error.to_string(),
            ),
        }
    });
}

#[cfg(test)]
mod tests;

/// Bundled language marks, supplied to the application's existing asset source.
pub fn marks() -> impl Iterator<Item = (&'static str, &'static [u8])> {
    [
        (
            "projects/languages/python.svg",
            include_bytes!("../assets/languages/python.svg").as_slice(),
        ),
        (
            "projects/languages/rust.svg",
            include_bytes!("../assets/languages/rust.svg").as_slice(),
        ),
        (
            "projects/languages/bun.svg",
            include_bytes!("../assets/languages/bun.svg").as_slice(),
        ),
        (
            "projects/languages/nodejs.svg",
            include_bytes!("../assets/languages/nodejs.svg").as_slice(),
        ),
        (
            "projects/languages/java.svg",
            include_bytes!("../assets/languages/java.svg").as_slice(),
        ),
    ]
    .into_iter()
}
