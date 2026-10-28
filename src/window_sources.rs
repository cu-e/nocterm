//! What a window is composed from: the connections, terminal capability, vault and settings
//! pages of its source.
//!
//! The application installs those once; a window takes the installation of its own source,
//! so a window never reaches for the ones of another.

use std::rc::Rc;

use crate::settings_pages::SettingsPages;

/// The installations one window is registered from.
#[derive(Clone)]
pub(crate) struct WindowSources {
    connections: Option<nocterm_connections::Installed>,
    terminal: nocterm_terminal::Installed,
    vault: Option<Rc<nocterm_vault_ui::Installed>>,
    settings: SettingsPages,
}

impl WindowSources {
    pub(crate) fn new(
        connections: Option<nocterm_connections::Installed>,
        terminal: nocterm_terminal::Installed,
        vault: Option<Rc<nocterm_vault_ui::Installed>>,
        settings: SettingsPages,
    ) -> Self {
        Self {
            connections,
            terminal,
            vault,
            settings,
        }
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
