//! The panels the window menu lists.
use nocterm_plugin_model::ContributionId;
use nocterm_window_layout::ResolvedLayout;

use super::panel_groups::PanelGroups;
use super::registration::Registrations;

/// A panel the Window menu can toggle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelEntry {
    id: ContributionId,
    title: &'static str,
    open: bool,
    available: bool,
}

impl PanelEntry {
    /// The registered panels the layout places, one list per group in bar order. Panels in the
    /// tray and panels that were never registered are left out, and so are empty groups.
    pub(super) fn listing(
        layout: &ResolvedLayout,
        registrations: &Registrations,
        panels: &PanelGroups,
    ) -> Vec<Vec<Self>> {
        layout
            .groups()
            .map(|group| {
                group
                    .panels()
                    .filter_map(|id| {
                        let contribution = registrations.registered_contribution(id)?;
                        Some(Self {
                            id: id.clone(),
                            title: contribution.title,
                            open: panels.panel_is_open(id),
                            available: panels.panel_is_available(id),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|entries| !entries.is_empty())
            .collect()
    }

    pub fn id(&self) -> &ContributionId {
        &self.id
    }

    pub fn title(&self) -> &'static str {
        self.title
    }

    /// Whether the panel shows now.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether the host offers the panel.
    pub fn is_available(&self) -> bool {
        self.available
    }
}
