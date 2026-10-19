//! The Explorer's place in the workspace: its sidebar panel, the transfer
//! status and the commands that show them.
use gpui_kit::{App, AppContext as _, Context, Window};
use nocterm_workspace::Workspace;

use crate::{Deps, FilesPanel, transfers};

gpui_kit::actions!(
    files,
    [
        /// Open the Explorer's settings: folder sizes and the programs that
        /// open files.
        OpenExplorerSettings,
        /// Open transfers and their progress, errors and cancellation controls.
        ShowTransfers,
        /// Show the Explorer in the sidebar, or hide the sidebar if it shows it.
        ToggleExplorer,
    ]
);

/// Installed handles retained by composition and copied into each window.
#[derive(Clone)]
pub struct Installed {
    deps: Deps,
    settings: nocterm_store::SettingsHandle,
}
impl Installed {
    pub(crate) fn settings(&self) -> &nocterm_store::SettingsHandle {
        &self.settings
    }
    pub(crate) fn local(&self) -> &std::sync::Arc<dyn nocterm_session::local_fs::LocalFs> {
        &self.deps.local
    }
    pub(crate) fn mutations(
        &self,
    ) -> &std::sync::Arc<dyn nocterm_session::local_fs::LocalMutations> {
        &self.deps.mutations
    }
    pub(crate) fn transfers(&self) -> &std::sync::Arc<dyn nocterm_transfers::TransferService> {
        &self.deps.transfers
    }
    pub(crate) fn opener(&self) -> &std::sync::Arc<dyn nocterm_session::ProgramOpener> {
        &self.deps.opener
    }

    /// Composition registers this independent future with its common shutdown
    /// coordinator. Admission closes synchronously before the future is polled.
    pub fn freeze_transfers(
        &self,
    ) -> futures::future::BoxFuture<'static, Result<(), nocterm_transfers::TransferError>> {
        self.deps.transfers.freeze_and_drain()
    }
}
pub fn install(deps: Deps, ready: &nocterm_ui::UiReady, cx: &mut App) -> Installed {
    ready.settings().register::<crate::ExplorerSettings>(cx);
    ready
        .settings()
        .register::<nocterm_session::LocalShellSettings>(cx);
    crate::path_input::install_bindings(cx);
    Installed {
        deps,
        settings: ready.settings().clone(),
    }
}
pub fn register(
    installed: &Installed,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let active = workspace.active_session(cx);
    let handle = cx.entity();
    let panel = cx.new(|cx| FilesPanel::new(installed.clone(), handle, active, window, cx));
    workspace.add_panel(panel, cx);
    let status =
        cx.new(|cx| transfers::TransferStatus::new(installed.deps.transfers.clone(), window, cx));
    workspace.add_status_view(status, cx);
    workspace.register_action(|workspace, _: &ToggleExplorer, window, cx| {
        workspace.toggle_panel_of::<FilesPanel>(window, cx);
    });
    let service = installed.deps.transfers.clone();
    workspace.register_action(move |workspace, _: &ShowTransfers, window, cx| {
        if let Some(view) = workspace.find_item::<transfers::TransfersView>() {
            workspace.activate_item_by_id(view.entity_id(), window, cx);
        } else {
            let handle = cx.entity().downgrade();
            let view = cx.new(|cx| transfers::TransfersView::new(service.clone(), handle, cx));
            workspace.add_item(view, window, cx);
        }
    });
}
