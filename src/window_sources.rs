//! What a window is composed from: the connections, terminal capability, vault and settings
//! pages of its environment.
//!
//! The application installs those once per environment; a window takes the installation of its
//! own, so a window never reaches for the ones of another.

use std::{rc::Rc, sync::Arc};

use nocterm_hubs_model::{Environment, HubError};
use nocterm_settings_ui::SettingsCatalogError;

use crate::settings_pages::SettingsPages;

/// The installations one window is registered from.
#[derive(Clone)]
pub(crate) struct WindowSources {
    environment: Environment,
    _hub_source: Option<crate::hub_sources::HubSource>,
    connections: Option<nocterm_connections::Installed>,
    terminal: nocterm_terminal::Installed,
    vault: Option<Rc<nocterm_vault_ui::Installed>>,
    settings: SettingsPages,
    agent_source: Option<Arc<dyn nocterm_hubs_model::source_identity::LocalAccountSourceFactory>>,
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
            agent_source: None,
            _hub_source: None,
        }
    }

    pub(crate) fn with_hub_source(mut self, source: crate::hub_sources::HubSource) -> Self {
        self.agent_source = Some(source.agent_source());
        self._hub_source = Some(source);
        self
    }
    pub(crate) fn agent_installation(
        &self,
        installed: &nocterm_agent::Installed,
    ) -> nocterm_agent::Installed {
        match (&self.environment, &self.agent_source) {
            (Environment::Hub(hub), Some(source)) => installed
                .with_account_source(source.clone())
                .for_hub_window(hub),
            (_, Some(source)) => installed.with_account_source(source.clone()),
            (Environment::Local, None) => installed.clone(),
            (Environment::Hub(hub), None) => installed.unavailable_account().for_hub_window(hub),
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
