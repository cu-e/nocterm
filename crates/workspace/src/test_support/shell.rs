//! Semantic names for the shell's footer toggles.
//!
//! Tests name an element through these helpers and not through its element id,
//! so a change of the production ids only changes this file.
use crate::Workspace;
use gpui_kit::{App, ElementId, SharedString};
use nocterm_ui::IconName;

/// A footer toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// The nth navigation panel.
    Navigation(usize),
    LocalTerminal,
    SidePanel,
}

/// The element id of a footer toggle.
pub fn toggle_id(toggle: Toggle) -> ElementId {
    match toggle {
        Toggle::Navigation(ix) => ("sidebar-panel", ix).into(),
        Toggle::LocalTerminal => "toggle-local-terminal".into(),
        Toggle::SidePanel => "toggle-right-panel".into(),
    }
}

/// The tooltip and icon a footer toggle is drawn with; `None` while it is not drawn.
pub fn toggle_presentation(
    workspace: &Workspace,
    toggle: Toggle,
    cx: &App,
) -> Option<(SharedString, IconName)> {
    let looks = workspace.footer_toggle_looks(cx);
    let look = match toggle {
        Toggle::Navigation(ix) => looks.navigation.get(ix).cloned(),
        Toggle::LocalTerminal => Some(looks.local_terminal),
        Toggle::SidePanel => looks.side_panel,
    }?;
    Some((look.tooltip, look.icon))
}
