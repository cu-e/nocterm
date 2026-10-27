//! The hubs installation windows are composed from.

use gpui_kit::{App, Entity};
use nocterm_hubs::Hubs;
use nocterm_hubs_model::HubId;

/// The registry of known hubs, kept by the application and given to each window.
#[derive(Clone)]
pub struct Installed {
    hubs: nocterm_hubs::Installed,
}

impl Installed {
    /// The name the user gave the server, or `None` when it is not in the list.
    pub fn label(&self, hub: &HubId, cx: &App) -> Option<String> {
        self.hubs()
            .read(cx)
            .summary(hub)
            .map(|summary| summary.label)
    }

    pub(crate) fn hubs(&self) -> &Entity<Hubs> {
        self.hubs.hubs()
    }
}

/// Keeps the service installation for the views of every window.
pub fn install(hubs: nocterm_hubs::Installed) -> Installed {
    Installed { hubs }
}
