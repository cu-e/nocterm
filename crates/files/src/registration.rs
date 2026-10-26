//! The Explorer's place in the workspace: its sidebar panel, the transfer
//! status and the commands that show them.
use gpui_kit::{App, AppContext as _, Context, Window};
use nocterm_workspace::{PanelRegistration, RegistrationError, WidgetRegistration, Workspace};

use crate::{Deps, FilesPanel, plugin, transfers};

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
    pub(crate) fn directories(
        &self,
    ) -> &std::sync::Arc<dyn nocterm_explorer::directory::DirectoryReaders> {
        &self.deps.directories
    }
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
    Installed {
        deps,
        settings: ready.settings().clone(),
    }
}
/// Adds the Explorer panel, the transfer status and the commands to a workspace.
///
/// The commands are registered even when the workspace refuses a contribution.
pub fn register(
    installed: &Installed,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<(), RegistrationError> {
    workspace.register_action(|workspace, _: &ToggleExplorer, window, cx| {
        workspace.toggle_panel(&plugin::EXPLORER.id, window, cx);
    });
    let service = installed.deps.transfers.clone();
    workspace.register_action(move |workspace, _: &ShowTransfers, window, cx| {
        if let Some(view) = workspace.find_item::<transfers::TransfersView>(cx) {
            workspace.activate_item_by_id(view.entity_id(), window, cx);
        } else {
            let handle = cx.entity().downgrade();
            let view = cx.new(|cx| transfers::TransfersView::new(service.clone(), handle, cx));
            workspace.add_item(view, window, cx);
        }
    });
    let panel_installed = installed.clone();
    let panel = workspace.register_panel(
        PanelRegistration::new(&plugin::EXPLORER, move |workspace, window, cx| {
            let handle = cx.entity();
            let active = workspace.explorer_source(cx);
            cx.new(|cx| FilesPanel::new(panel_installed.clone(), handle, active, window, cx))
        }),
        window,
        cx,
    );
    let transfers = installed.deps.transfers.clone();
    let status = workspace.register_widget(
        WidgetRegistration::new(&plugin::TRANSFERS, move |_, window, cx| {
            let transfers = transfers.clone();
            cx.new(|cx| transfers::TransferStatus::new(transfers, window, cx))
        }),
        window,
        cx,
    );
    panel.and(status)
}
