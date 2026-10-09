//! The Explorer's place in the workspace: its sidebar panel, the transfer
//! status and the commands that show them.
use gpui_kit::{AppContext as _, Context, Window};
use nocterm_workspace::Workspace;

use crate::{FilesPanel, transfers};

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

pub fn register(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    nocterm_ui::register_setting::<crate::ExplorerSettings>(cx);
    transfers::init(cx);
    let active = workspace.active_session(cx);
    let handle = cx.entity();
    let panel = cx.new(|cx| FilesPanel::new(handle, active, window, cx));
    workspace.add_panel(panel, cx);
    let status = cx.new(|cx| transfers::TransferStatus::new(window, cx));
    workspace.add_status_view(status, cx);
    workspace.register_action(|workspace, _: &ToggleExplorer, window, cx| {
        workspace.toggle_panel_of::<FilesPanel>(window, cx);
    });
    workspace.register_action(|workspace, _: &ShowTransfers, window, cx| {
        if let Some(view) = workspace.find_item::<transfers::TransfersView>() {
            workspace.activate_item_by_id(view.entity_id(), window, cx);
        } else {
            let handle = cx.entity().downgrade();
            let view = cx.new(|cx| transfers::TransfersView::new(handle, cx));
            workspace.add_item(view, window, cx);
        }
    });
}
