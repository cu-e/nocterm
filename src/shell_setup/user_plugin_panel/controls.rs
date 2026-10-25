//! What a plugin may do to the badge of its own panel. Opening and closing the panel is the
//! window's (`ShellCapabilities`), which enforces the shell's rules; the host has checked the
//! grant, the rate, the gesture and the surface before it asks for a badge.
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui_kit::App;
use gpui_kit::WeakEntity;
use nocterm_plugin_api::{
    HostError,
    capability::panels::{SetBadge, SetPanelBadge},
};
use nocterm_plugin_model::{ContributionId, PluginId};
use nocterm_plugin_ports::Apply;

use super::views::UserPluginPanel;

/// The panels of a window that can carry a badge, by contribution.
#[derive(Clone, Default)]
pub(in crate::shell_setup) struct Badges(
    Rc<RefCell<HashMap<ContributionId, WeakEntity<UserPluginPanel>>>>,
);

impl Badges {
    /// Remembers the panel made for the contribution, and forgets the panels that are gone.
    pub(super) fn add(&self, id: ContributionId, panel: WeakEntity<UserPluginPanel>) {
        let mut panels = self.0.borrow_mut();
        panels.retain(|_, known| known.upgrade().is_some());
        panels.insert(id, panel);
    }

    fn get(&self, id: &ContributionId) -> Option<WeakEntity<UserPluginPanel>> {
        self.0.borrow().get(id).cloned()
    }
}

impl Apply<SetPanelBadge> for Badges {
    fn apply(&self, plugin: &PluginId, request: SetBadge, cx: &mut App) -> Result<(), HostError> {
        let id = ContributionId::in_plugin(plugin, request.local.as_str())
            .map_err(|_| HostError::Unavailable)?;
        let panel = self
            .get(&id)
            .and_then(|panel| panel.upgrade())
            .ok_or(HostError::Unavailable)?;
        panel.update(cx, |panel, cx| panel.set_badge(request.badge, cx));
        Ok(())
    }
}
