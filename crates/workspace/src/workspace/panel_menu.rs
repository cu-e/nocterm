//! The panels the window menu lists.
use nocterm_plugin_model::{ContributionId, PluginSet, UserAvailability};
use nocterm_window_layout::ResolvedLayout;

use super::panel_groups::PanelGroups;
use super::registration::Registrations;

/// A panel the Window menu can toggle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelEntry {
    id: ContributionId,
    title: String,
    /// The name of the user plugin that owns the panel; `None` for the shell and system plugins.
    user_plugin: Option<String>,
    /// Whether the run is a developer session, whose user plugin names end in DEV.
    developer: bool,
    open: bool,
    available: bool,
}

impl PanelEntry {
    /// The registered panels the layout places, one list per group in bar order. Panels in the
    /// tray and panels that were never registered are left out, and so are empty groups.
    pub(super) fn listing(
        layout: &ResolvedLayout,
        set: &PluginSet,
        registrations: &Registrations,
        panels: &PanelGroups,
    ) -> Vec<Vec<Self>> {
        layout
            .groups()
            .map(|group| {
                group
                    .panels()
                    .filter_map(|id| {
                        let contribution = registrations.registered_contribution(set, id)?;
                        Some(Self {
                            id: id.clone(),
                            title: contribution.title().to_owned(),
                            user_plugin: contribution
                                .is_user()
                                .then(|| set.plugin_of(id).map(|plugin| plugin.name().to_owned()))
                                .flatten(),
                            developer: set.availability() == UserAvailability::Developer,
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

    pub fn title(&self) -> &str {
        &self.title
    }

    /// The name of the user plugin that owns the panel, if it is one.
    pub fn user_plugin(&self) -> Option<&str> {
        self.user_plugin.as_deref()
    }

    /// What menus and the palette call the panel: a user plugin's panel is named after its
    /// plugin, so it cannot pass for one of the shell's own, and a developer session's ends in
    /// DEV.
    pub fn label(&self) -> String {
        match (&self.user_plugin, self.developer) {
            (Some(name), true) => nocterm_plugin_model::developer_label(name, &self.title),
            (Some(name), false) => nocterm_plugin_model::user_label(name, &self.title),
            (None, _) => self.title.clone(),
        }
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
