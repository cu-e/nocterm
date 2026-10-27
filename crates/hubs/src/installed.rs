//! The ports the service needs and the handle to the installed registry.

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity};
use nocterm_hubs_model::{HubSessions, HubSignIn, HubTrust};
use nocterm_store::SettingsHandle;

use crate::registry::Hubs;

/// The hub access ports, supplied by the composition root.
#[derive(Clone)]
pub struct Deps {
    pub trust: Arc<dyn HubTrust>,
    pub sign_in: Arc<dyn HubSignIn>,
    pub sessions: Arc<dyn HubSessions>,
}

/// The registry of known hubs, kept alive by whoever composes the application.
#[derive(Clone)]
pub struct Installed {
    hubs: Entity<Hubs>,
}

impl Installed {
    pub fn hubs(&self) -> &Entity<Hubs> {
        &self.hubs
    }
}

/// Registers the `[hubs]` settings section and starts connecting to every known hub.
pub fn install(deps: Deps, settings: SettingsHandle, cx: &mut App) -> Installed {
    Installed {
        hubs: cx.new(|cx| Hubs::new(deps, settings, cx)),
    }
}
