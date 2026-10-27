//! The hubs installation windows are composed from.

use gpui_kit::Entity;
use nocterm_hubs::Hubs;

/// The registry of known hubs, kept by the application and given to each window.
#[derive(Clone)]
pub struct Installed {
    hubs: nocterm_hubs::Installed,
}

impl Installed {
    pub(crate) fn hubs(&self) -> &Entity<Hubs> {
        self.hubs.hubs()
    }
}

/// Keeps the service installation for the views of every window.
pub fn install(hubs: nocterm_hubs::Installed) -> Installed {
    Installed { hubs }
}
