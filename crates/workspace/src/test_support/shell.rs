//! Semantic names for the shell's status bar toggles.
//!
//! Tests name an element through these helpers and not through its element id,
//! so a change of the production ids only changes this file.
use crate::Workspace;
use gpui_kit::{App, Context, ElementId, SharedString};
use nocterm_ui::IconName;

/// A status bar toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// The nth navigation panel.
    Navigation(usize),
    LocalTerminal,
    /// The first panel of the group beside the sidebar's.
    SidePanel,
}

/// The element id of a status bar toggle.
pub fn toggle_id(toggle: Toggle) -> ElementId {
    match toggle {
        Toggle::Navigation(ix) => ("sidebar-panel", ix).into(),
        Toggle::LocalTerminal => {
            (ElementId::from("status-item"), "workspace.local-terminal").into()
        }
        Toggle::SidePanel => (ElementId::from("panel-group-toggle"), "assistant:0").into(),
    }
}

/// The tooltip and icon a status bar toggle is drawn with; `None` while it is not drawn.
pub fn toggle_presentation(
    workspace: &Workspace,
    toggle: Toggle,
    cx: &App,
) -> Option<(SharedString, IconName)> {
    let looks = workspace.toggle_looks(cx);
    let look = match toggle {
        Toggle::Navigation(ix) => looks.navigation.get(ix).cloned(),
        Toggle::LocalTerminal => looks.local_terminal,
        Toggle::SidePanel => looks.side_panel,
    }?;
    Some((look.tooltip, look.icon))
}

/// Makes the side panel ask for the user, or stop asking, as an agent waiting for a permission
/// does.
pub fn set_side_panel_attention(
    workspace: &mut Workspace,
    attention: bool,
    cx: &mut Context<Workspace>,
) {
    workspace.set_side_panel_attention(attention, cx);
}
