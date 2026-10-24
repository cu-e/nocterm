//! Contextual snippet library in the workspace sidebar.
//! A checked run target consumes its original terminal facts without exposing mutable paste authority.
//!
//! Owns: retained service/settings/identity capabilities and editor/panel presentation.
//! SnippetDraft owns literal native controls; AttachmentPicker owns search/folding/section state.
//! Does not own: checked record rules or ordered background write orchestration.
//! Native storage/entropy is supplied by composition; the library model lives in snippets-store.
//! Ports used: SnippetsHandle, SettingsHandle, SnippetIds, UserTerminalPaste.
//! Ports provided: the `snippets` system plugin (`plugin::PLUGIN`) with its sidebar panel.
//! Settings section: reads terminal through supplied SettingsHandle. Globals: none.
//! Invariants: failed persistence never publishes a change; malformed libraries
//! remain read-only; final accepted work uses the shared shutdown coordinator.
//! Snippet submission keeps target and terminal refusals typed until notice presentation.
//! Read-only diagnostics share their original non-Clone load error through core SharedError.

mod editor;
mod installation;
pub mod plugin;
#[cfg(test)]
mod test_support;
pub use installation::Installed;
mod panel;
use gpui_kit::{App, AppContext as _, Context, Window};
pub use nocterm_snippets_store::{SnippetsError, SnippetsLoadError};
use nocterm_workspace::{PanelRegistration, RegistrationError, Workspace};
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
/// Retain the actual installed library, original settings and draft identity capability.
pub fn init(
    model: nocterm_snippets_store::SnippetsHandle,
    settings: nocterm_store::SettingsHandle,
    ids: std::sync::Arc<dyn nocterm_snippets::SnippetIds>,
    cx: &mut App,
) -> Installed {
    settings.register::<nocterm_ui::TerminalSettings>(cx);
    Installed::new(model, settings, ids)
}
/// Register the snippet switch and action in a workspace.
///
/// The actions are registered even when the workspace refuses the panel.
pub fn register(
    workspace: &mut Workspace,
    installed: &Installed,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), RegistrationError> {
    let editors = installed.clone();
    workspace.register_action(move |_, _: &NewSnippet, window, cx| {
        editor::open(editors.clone(), None, cx.entity().downgrade(), window, cx);
    });
    workspace.register_action(|workspace, _: &ToggleSnippets, window, cx| {
        workspace.toggle_panel(&plugin::LIBRARY.id, window, cx)
    });
    let installed = installed.clone();
    workspace.register_panel(
        PanelRegistration::new(&plugin::LIBRARY, move |_, window, cx| {
            let handle = cx.entity();
            cx.new(|cx| SnippetsPanel::new(installed.clone(), handle.clone(), window, cx))
        }),
        window,
        cx,
    )
}
