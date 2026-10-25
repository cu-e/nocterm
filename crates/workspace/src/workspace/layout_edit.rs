//! Writing edits of the stored layouts: the one path the status bar menu and the layout editor
//! share.
//!
//! An edit is applied by `nocterm_window_layout::edit` against the document as it is when the
//! ordered write runs, so edits queued behind one another compose. A `[layouts]` table that does
//! not read is never overwritten.

use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui_kit::{App, Context, Task, Window};
use nocterm_plugin_model::{ContributionId, PluginSet};
use nocterm_store::{SettingsHandle, StoreError};
use nocterm_ui::{LayoutHandle, notice};
use nocterm_window_layout::{EditError, EditOp, EditOutcome, EditTarget, LayoutSettings, edit};

use super::{Workspace, layout_switch::MalformedLayouts};

/// Whether the sidebar and the side panel traded places, before layouts were stored.
const LEGACY_SWAPPED: &str = "workspace.swapped";

/// The retired swap-sides flag, which still picks the Swapped sides layout for a user who never
/// chose one. Windows read it once, when they are composed.
pub fn legacy_layout_swapped(flags: &LayoutHandle, cx: &App) -> bool {
    flags.flag(LEGACY_SWAPPED, cx)
}

/// Removes the item from the bar of the selected layout from a command, as the status bar menu
/// does, and tells the user when it fails.
pub(super) fn hide(
    workspace: &mut Workspace,
    id: ContributionId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let task = workspace
        .shell
        .edit(EditTarget::Selected, EditOp::RemoveToTray { id }, cx);
    window
        .spawn(cx, async move |cx| {
            let result = task.await;
            let _ = cx.update(|window, cx| match result {
                Ok(_) => notice::remove(window, cx, "layout-edit"),
                Err(error) => notice::error(
                    window,
                    cx,
                    "layout-edit",
                    "Could not change the layout",
                    error.to_string(),
                ),
            });
        })
        .detach();
}

/// Why an edit of the layouts did not happen. The stored layouts are unchanged.
#[derive(Debug, thiserror::Error)]
pub enum LayoutEditError {
    /// The `[layouts]` table of the settings file does not read.
    #[error(transparent)]
    Malformed(#[from] MalformedLayouts),
    /// The rules refuse the edit.
    #[error(transparent)]
    Refused(#[from] EditError),
    #[error(transparent)]
    Store(StoreError),
}

impl From<StoreError> for LayoutEditError {
    fn from(error: StoreError) -> Self {
        let StoreError::Edit(cause) = &error else {
            return Self::Store(error);
        };
        if let Some(malformed) = cause.downcast_ref::<MalformedLayouts>() {
            return Self::Malformed(malformed.clone());
        }
        match cause.downcast_ref::<EditError>() {
            Some(refused) => Self::Refused(refused.clone()),
            None => Self::Store(error),
        }
    }
}

/// Applies `op` to the layout `target` names through the ordered settings queue.
///
/// Resolves to the outcome once the edit is published. `None` means the write was accepted
/// without the edit running, which a store that drops the change reports.
pub fn edit_layouts(
    settings: &SettingsHandle,
    catalog: Arc<PluginSet>,
    legacy_swapped: bool,
    target: EditTarget,
    op: EditOp,
    cx: &mut App,
) -> Task<Result<Option<EditOutcome>, LayoutEditError>> {
    let outcome = Rc::new(Cell::new(None));
    let slot = outcome.clone();
    let write = settings.edit_checked(cx, move |document| {
        document.register::<LayoutSettings>();
        if let Some(reason) = LayoutSettings::read_error(document) {
            return Err(StoreError::edit(MalformedLayouts::new(reason)));
        }
        let mut layouts = document.get::<LayoutSettings>().clone();
        let done =
            edit(&mut layouts, legacy_swapped, &catalog, target, op).map_err(StoreError::edit)?;
        document.set(layouts);
        slot.set(Some(done));
        Ok(())
    });
    cx.spawn(async move |_| {
        write.await?;
        Ok(outcome.take())
    })
}
