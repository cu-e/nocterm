//! Folder workspaces grouped by their original ordinary connection.
//!
//! Owns: saved folder library, name/path editor, one outer workspace Item with
//! inner terminal tabs, and automatic ordinary connection resolution.
//! Does not own: sessions, transport I/O, catalog persistence or chat documents.
//! Ports used: ProjectsHandle, original ProjectTerminalSession and normal
//! ProjectTerminalResolve. Ports provided: Installed registration and ProjectView.
//! Settings section: none. Globals: none.
//! Invariants: original parent/route and ProjectId remain stable; delayed recovery
//! rechecks its original binding/claim and never steals changed focus.

mod context;
mod editor;
mod error;
mod installation;
mod opening;
mod panel;
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
        /// Create a workspace on the current ordinary connection.
        NewProject,
    ]
);

pub fn init(
    model: nocterm_project_service::ProjectsHandle,
    terminals: std::rc::Rc<dyn nocterm_workspace::ProjectTerminalOpen>,
) -> Installed {
    Installed::new(model, terminals)
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
        match context::capture_current(workspace, window, cx) {
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
