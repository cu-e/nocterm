//! What a window is composed from: the connections, terminal capability, vault and settings
//! pages of its environment.
//!
//! The application installs those once per environment; a window takes the installation of its
//! own, so a window never reaches for the ones of another.

use std::rc::Rc;

use nocterm_hubs_model::{Environment, HubError};
use nocterm_settings_ui::SettingsCatalogError;

use crate::settings_pages::SettingsPages;

/// The installations one window is registered from.
#[derive(Clone)]
pub(crate) struct WindowSources {
    environment: Environment,
    connections: Option<nocterm_connections::Installed>,
    terminal: nocterm_terminal::Installed,
    vault: Option<Rc<nocterm_vault_ui::Installed>>,
    settings: SettingsPages,
}

/// Why a window cannot be composed for an environment.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WindowSourcesError {
    #[error("this application has no access to hub accounts")]
    NoHubs,
    #[error("the account of the hub cannot be opened")]
    Account(#[from] HubError),
    #[error("could not compose the settings pages of the window")]
    Pages(#[from] SettingsCatalogError),
}

impl WindowSources {
    pub(crate) fn new(
        environment: Environment,
        connections: Option<nocterm_connections::Installed>,
        terminal: nocterm_terminal::Installed,
        vault: Option<Rc<nocterm_vault_ui::Installed>>,
        settings: SettingsPages,
    ) -> Self {
        Self {
            environment,
            connections,
            terminal,
            vault,
            settings,
        }
    }

    pub(crate) fn environment(&self) -> &Environment {
        &self.environment
    }

    pub(crate) fn connections(&self) -> Option<&nocterm_connections::Installed> {
        self.connections.as_ref()
    }

    pub(crate) fn terminal(&self) -> &nocterm_terminal::Installed {
        &self.terminal
    }

    pub(crate) fn vault(&self) -> Option<&Rc<nocterm_vault_ui::Installed>> {
        self.vault.as_ref()
    }

    pub(crate) fn settings(&self) -> &SettingsPages {
        &self.settings
    }
}
