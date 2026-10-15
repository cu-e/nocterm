//! Contextual snippet library in the workspace sidebar.
//!
//! Owns: retained library/settings installation, editor/panel presentation and identity generation.
//! SnippetDraft owns literal native controls; AttachmentPicker owns search/folding/section state.
//! Does not own: checked record rules or ordered background write orchestration.
//! Existing native library loading/identity creation remain explicit feature migration debt.
//! Ports used: SnapshotWriter, UserTerminalPaste. Ports provided: sidebar panel.
//! Settings section: reads terminal through supplied SettingsHandle. Globals: none.
//! Invariants: failed persistence never publishes a change; malformed libraries
//! remain read-only; final accepted work uses the shared shutdown coordinator.
//! Snippet submission keeps target and terminal refusals typed until notice presentation.
//! Read-only diagnostics share their original non-Clone load error through core SharedError.

mod editor;
mod installation;
mod model;
pub use installation::Installed;
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
    settings: nocterm_store::SettingsHandle,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> Result<Installed, nocterm_store::StoreError> {
    let model = model::init(paths, shutdown, cx)?;
    Ok(Installed::new(model, settings))
}
/// Register the snippet switch and action in a workspace.
pub fn register(
    workspace: &mut Workspace,
    installed: &Installed,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let handle = cx.entity();
    let installed = installed.clone();
    let panel = cx.new(|cx| SnippetsPanel::new(installed.clone(), handle.clone(), window, cx));
    workspace.add_panel(panel, cx);
    workspace.register_action(move |_, _: &NewSnippet, window, cx| {
        editor::open(installed.clone(), None, cx.entity().downgrade(), window, cx);
    });
    workspace.register_action(|workspace, _: &ToggleSnippets, window, cx| {
        workspace.toggle_panel_of::<SnippetsPanel>(window, cx)
    });
}
