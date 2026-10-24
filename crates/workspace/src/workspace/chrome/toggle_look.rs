//! What a status bar toggle shows: its icon and its tooltip.
//!
//! The bar draws every panel toggle from one of these, so the presentation is defined in a
//! single place and can be observed without reading pixels.

use gpui_kit::{App, SharedString};
use nocterm_plugin_model::SystemContribution;
use nocterm_ui::{IconName, contribution_icon};

use crate::workspace::panel_groups::HostedPanel;

/// The icon and tooltip of one toggle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToggleLook {
    pub(crate) icon: IconName,
    pub(crate) tooltip: SharedString,
}

impl ToggleLook {
    /// A toggle shows its contribution's title and icon. While the panel asks for the user it
    /// shows the shield and says so, whether or not the panel is open.
    pub(super) fn of(contribution: &SystemContribution, attention: bool) -> Self {
        if attention {
            Self {
                icon: IconName::ShieldCheck,
                tooltip: format!("{}: permission required", contribution.title).into(),
            }
        } else {
            Self {
                icon: contribution_icon(contribution.icon),
                tooltip: contribution.title.into(),
            }
        }
    }

    /// The toggle of a sidebar panel that the pre-plugin API added, which has no contribution:
    /// the panel's own title and icon.
    pub(super) fn legacy_navigation(entry: &HostedPanel, cx: &App) -> Self {
        Self {
            icon: entry.icon(cx),
            tooltip: entry.title(cx),
        }
    }

    /// The toggle of a side panel that the pre-plugin API added.
    pub(super) fn legacy_side_panel(attention: bool) -> Self {
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
