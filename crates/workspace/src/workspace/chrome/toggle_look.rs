//! What a footer toggle shows: its icon and its tooltip.
//!
//! The footer draws every toggle from one of these, so the presentation is
//! defined in a single place and can be observed without reading pixels.

use gpui_kit::{App, SharedString};
use nocterm_ui::IconName;

use crate::workspace::panels::{Sidebar, SidebarEntry};
use crate::workspace::right_panel::RightPanelSlot;

/// The icon and tooltip of one footer toggle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToggleLook {
    pub(crate) icon: IconName,
    pub(crate) tooltip: SharedString,
}

impl ToggleLook {
    pub(super) fn navigation(entry: &SidebarEntry, cx: &App) -> Self {
        Self {
            icon: entry.icon(cx),
            tooltip: entry.title(cx),
        }
    }

    pub(super) fn local_terminal() -> Self {
        Self {
            icon: IconName::SquareTerminal,
            tooltip: "Local Terminal".into(),
        }
    }

    pub(super) fn side_panel(attention: bool) -> Self {
        if attention {
            Self {
                icon: IconName::ShieldCheck,
                tooltip: "AI Agents: permission required".into(),
            }
        } else {
            Self {
                icon: IconName::PanelRight,
                tooltip: "AI Agents".into(),
            }
        }
    }
}

/// The presentation of every footer toggle.
///
/// The footer draws from one value of this type, and tests read the same value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FooterLooks {
    pub(crate) navigation: Vec<ToggleLook>,
    pub(crate) local_terminal: ToggleLook,
    /// `None` while no side panel is installed or it is unavailable.
    pub(crate) side_panel: Option<ToggleLook>,
}

impl FooterLooks {
    pub(in crate::workspace) fn of(sidebar: &Sidebar, right: &RightPanelSlot, cx: &App) -> Self {
        Self {
            navigation: sidebar
                .entries()
                .map(|entry| ToggleLook::navigation(entry, cx))
                .collect(),
            local_terminal: ToggleLook::local_terminal(),
            side_panel: right
                .available()
                .then(|| ToggleLook::side_panel(right.attention())),
        }
    }
}
