//! Contextual snippet library in the workspace sidebar.
//!
//! Owns: library installation, editor/panel presentation and identity generation.
//! Does not own: checked record rules or ordered background write orchestration.
//! Ports used: SnapshotWriter, TerminalAccess. Ports provided: sidebar panel.
//! Settings section: none. Globals: GlobalLibrary (temporary compatibility).
//! Invariants: failed persistence never publishes a change; malformed libraries
//! remain read-only; final accepted work uses the shared shutdown coordinator.
mod editor;
mod model;
mod panel;
use gpui_kit::{App, AppContext as _, Context, Window};
pub use model::{SnippetsError, SnippetsLoadError};
use nocterm_core::Paths;
use nocterm_workspace::Workspace;
pub use panel::SnippetsPanel;

gpui_kit::actions!(
    snippets,
    [
        /// Show the snippet library in the sidebar, or hide it.
        ToggleSnippets,
        /// Create a snippet in a focused editor.
        NewSnippet,
    ]
);
/// Install the shared library; `None` provides in-memory storage for tests.
pub fn init(
    paths: Option<&Paths>,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> Result<(), nocterm_store::StoreError> {
    model::init(paths, shutdown, cx)
}
/// Register the snippet switch and action in a workspace.
pub fn register(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let handle = cx.entity();
    let model = model::Snippets::global(cx);
    let panel = cx.new(|cx| SnippetsPanel::new(model, handle.clone(), window, cx));
    workspace.add_panel(panel, cx);
    workspace.register_action(|_, _: &NewSnippet, window, cx| {
        editor::open(None, cx.entity().downgrade(), window, cx);
    });
    workspace.register_action(|workspace, _: &ToggleSnippets, window, cx| {
        workspace.toggle_panel_of::<SnippetsPanel>(window, cx)
    });
}
