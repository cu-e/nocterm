//! What a status bar toggle shows: its icon and its tooltip.
//!
//! The bar draws every panel toggle from one of these, so the presentation is defined in a
//! single place and can be observed without reading pixels.

use gpui_kit::SharedString;
use nocterm_plugin_model::{ContributionRef, Icon, SystemContribution};
use nocterm_ui::{IconName, contribution_icon};

/// The icon and tooltip of one toggle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToggleLook {
    pub(crate) icon: IconName,
    pub(crate) tooltip: SharedString,
}

impl ToggleLook {
    /// A toggle shows its contribution's title and icon. While the panel asks for the user it
    /// shows the shield and says so, whether or not the panel is open.
    pub(super) fn of(contribution: ContributionRef<'_>, attention: bool) -> Self {
        Self::titled(contribution.title(), contribution.icon(), attention)
    }

    /// The toggle of one of the shell's own contributions.
    pub(super) fn of_system(contribution: &SystemContribution, attention: bool) -> Self {
        Self::titled(contribution.title, contribution.icon, attention)
    }

    /// The toggle of a user plugin's panel, whose tooltip names the plugin.
    pub(super) fn of_user(
        contribution: ContributionRef<'_>,
        plugin: &str,
        attention: bool,
    ) -> Self {
        let label = nocterm_plugin_model::user_label(plugin, contribution.title());
        let mut look = Self::of(contribution, attention);
        look.tooltip = if attention {
            format!("{label}: permission required").into()
        } else {
            label.into()
        };
        look
    }

    fn titled(title: &str, icon: Icon, attention: bool) -> Self {
        if attention {
            Self {
                icon: IconName::ShieldCheck,
                tooltip: format!("{title}: permission required").into(),
            }
        } else {
            Self {
                icon: contribution_icon(icon),
                tooltip: title.to_owned().into(),
            }
        }
    }
}
