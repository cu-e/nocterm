//! Choosing the window layout: the layouts a user can pick, why a pick can fail, and the
//! notice and picker that go with it.
//!
//! The layouts are app-wide settings, so a switch made from any window reaches every window
//! through the settings handle; each one applies it to its own panels.
use super::*;
use gpui_kit::{Task, WeakEntity};
use nocterm_store::StoreError;
use nocterm_ui::{PickerOption, notice, open_choice_picker};
use nocterm_window_layout::PresetId;

/// A layout the user can switch to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutChoice {
    id: PresetId,
    name: String,
    selected: bool,
}

impl LayoutChoice {
    pub(super) fn new(id: PresetId, name: String, selected: bool) -> Self {
        Self { id, name, selected }
    }

    pub fn id(&self) -> &PresetId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the window shows this layout now.
    pub fn is_selected(&self) -> bool {
        self.selected
    }
}

/// Why a layout could not be selected.
#[derive(Debug, thiserror::Error)]
pub enum LayoutSwitchError {
    #[error("there is no layout \"{0}\"")]
    Unknown(PresetId),
    /// The `[layouts]` table of the settings file does not read. Selecting a layout would write
    /// the section again and replace the table, so the file is left as the user wrote it.
    #[error(transparent)]
    Malformed(#[from] MalformedLayouts),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// The `[layouts]` table could not be read, so nothing can be written to it.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the [layouts] table of settings.toml does not read ({reason}); fix it to switch layouts")]
pub struct MalformedLayouts {
    reason: String,
}

impl MalformedLayouts {
    pub(super) fn new(reason: String) -> Self {
        Self { reason }
    }

    /// What was wrong with the table.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// The layout was removed between the check and the ordered write.
#[derive(Debug, thiserror::Error)]
#[error("the layout \"{0}\" no longer exists")]
pub(super) struct Removed(pub(super) PresetId);

/// Selects a layout for every window from a command, lets this window follow at once and tells
/// the user when it fails. Settings without a writer publish at once, so the window does not
/// wait for the observation.
pub(crate) fn switch(
    workspace: &mut Workspace,
    id: PresetId,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let task = workspace.switch_layout(id, cx);
    workspace.layout_settings_changed(window, cx);
    report_switch(task, window, cx);
}

/// Switches to the layout `offset` places after the selected one in menu order, wrapping.
pub(super) fn cycle(
    workspace: &mut Workspace,
    offset: isize,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if let Some(id) = workspace.shell.neighbour(offset) {
        switch(workspace, id, window, cx);
    }
}

/// Tells the user when a switch fails, and clears an earlier notice when it succeeds.
fn report_switch(task: Task<Result<(), LayoutSwitchError>>, window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            let result = task.await;
            let _ = cx.update(|window, cx| match result {
                Ok(()) => notice::remove(window, cx, "layout-switch"),
                Err(error) => notice::error(
                    window,
                    cx,
                    "layout-switch",
                    "Could not switch layout",
                    error.to_string(),
                ),
            });
        })
        .detach();
}

/// Asks which layout to switch to, and switches the window's workspace to the answer.
pub(super) fn pick_layout(
    workspace: WeakEntity<Workspace>,
    choices: Vec<LayoutChoice>,
    window: &mut Window,
    cx: &mut App,
) {
    let options = choices
        .into_iter()
        .map(|choice| PickerOption::new(choice.name, choice.selected, choice.id))
        .collect();
    window.defer(cx, move |window, cx| {
        open_choice_picker(
            "Switch layout…",
            options,
            move |id: PresetId, window, cx| {
                let _ = workspace.update(cx, |this, cx| switch(this, id, window, cx));
            },
            window,
            cx,
        );
    });
}
