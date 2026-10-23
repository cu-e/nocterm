//! App-owned checked settings catalogs and native action routing.
use nocterm_settings_ui::{
    Installed, SettingsCatalog, SettingsCatalogError, SettingsPageRegistration,
};
use nocterm_ui::{SettingsPageSpec, ThemeUi};
use nocterm_workspace::{
    ChangeColorScheme, ChangeTheme, DefaultSessionSettings, OpenAiSettings, OpenKeymap,
    OpenSSHSettings, OpenVault, Workspace,
};

#[derive(Clone)]
pub(crate) struct SettingsPages {
    installed: Installed,
    ordinary: SettingsCatalog,
    startup: Option<SettingsCatalog>,
}
impl SettingsPages {
    /// Gather page registrations from the exact feature installations retained by Booted.
    pub(crate) fn from_installations(
        settings: nocterm_store::SettingsHandle,
        themes: ThemeUi,
        terminal: &nocterm_terminal::Installed,
        agent: &nocterm_agent::Installed,
        sandbox_probe: std::sync::Arc<dyn nocterm_ai::sandbox::SandboxProbe>,
        guests: Vec<SettingsPageSpec>,
        vault: Option<&nocterm_vault_ui::Installed>,
    ) -> Result<Self, SettingsCatalogError> {
        Self::compose(
            Installed::new(settings.clone()),
            themes,
            nocterm_terminal::settings_pages(terminal),
            nocterm_agent_settings_ui::settings_page(
                settings,
                agent.agents().clone(),
                sandbox_probe,
            ),
            guests,
            vault.map(nocterm_vault_ui::settings_page),
        )
    }

    pub(crate) fn compose(
        installed: Installed,
        themes: ThemeUi,
        terminal: [SettingsPageSpec; 3],
        ai: SettingsPageSpec,
        guests: Vec<SettingsPageSpec>,
        vault: Option<SettingsPageSpec>,
    ) -> Result<Self, SettingsCatalogError> {
        let [terminal, local, ssh] = terminal;
        let builtins = [
            nocterm_ui::appearance_settings_page(themes),
            terminal,
            local,
            ssh,
            ai,
        ];
        let guests = guests.into_iter().chain(vault.clone()).collect();
        Self::new(installed, builtins, guests, vault)
    }
    pub(crate) fn new(
        installed: Installed,
        builtins: [SettingsPageSpec; 5],
        guests: Vec<SettingsPageSpec>,
        startup_vault: Option<SettingsPageSpec>,
    ) -> Result<Self, SettingsCatalogError> {
        let ordinary = catalog(builtins.clone(), guests)?;
        let startup = startup_vault
            .map(|vault| catalog(builtins, vec![vault]))
            .transpose()?;
        Ok(Self {
            installed,
            ordinary,
            startup,
        })
    }
    pub(crate) fn installed(&self) -> Installed {
        self.installed.clone()
    }
    #[cfg(test)]
    pub(crate) fn ordinary(&self) -> &SettingsCatalog {
        &self.ordinary
    }
    pub(crate) fn startup(&self) -> Option<&SettingsCatalog> {
        self.startup.as_ref()
    }
    pub(crate) fn register(&self, workspace: &mut Workspace, themes: ThemeUi) {
        register_appearance_commands(workspace, themes, self.installed.settings().clone());
        nocterm_settings_ui::register(workspace, self.ordinary.clone(), self.installed.clone());
        open_on::<DefaultSessionSettings>(workspace, "ssh", &self.ordinary, self.installed.clone());
        open_on::<OpenAiSettings>(workspace, "ai", &self.ordinary, self.installed.clone());
        open_on::<OpenSSHSettings>(workspace, "ssh", &self.ordinary, self.installed.clone());
        open_on::<OpenKeymap>(workspace, "keymap", &self.ordinary, self.installed.clone());
        open_on::<nocterm_monitor_ui::OpenMonitorSettings>(
            workspace,
            "monitor",
            &self.ordinary,
            self.installed.clone(),
        );
        open_on::<nocterm_files::OpenExplorerSettings>(
            workspace,
            "explorer",
            &self.ordinary,
            self.installed.clone(),
        );
        if self.startup.is_some() {
            open_on::<OpenVault>(workspace, "vault", &self.ordinary, self.installed.clone());
        }
    }
}
/// Lazy pages are composed from the same feature installations as the window.
pub(crate) fn guest_pages(
    keymap: nocterm_keymap::KeymapHandle,
    monitor: &nocterm_monitor_ui::Installed,
    files: &nocterm_files::Installed,
    projects: Option<&nocterm_projects::Installed>,
) -> Vec<SettingsPageSpec> {
    let mut pages = vec![
        nocterm_keymap_ui::settings_page(keymap),
        nocterm_monitor_ui::settings_page(monitor),
        nocterm_files::settings_page(files),
    ];
    pages.extend(projects.map(nocterm_projects::settings_page));
    pages
}
fn catalog(
    builtins: [SettingsPageSpec; 5],
    guests: Vec<SettingsPageSpec>,
) -> Result<SettingsCatalog, SettingsCatalogError> {
    SettingsCatalog::new(
        builtins
            .into_iter()
            .map(SettingsPageRegistration::eager)
            .chain(guests.into_iter().map(SettingsPageRegistration::lazy))
            .collect(),
        "appearance",
    )
}
fn open_on<A: gpui_kit::Action>(
    workspace: &mut Workspace,
    id: &'static str,
    pages: &SettingsCatalog,
    installed: Installed,
) {
    let pages = pages.clone();
    workspace.register_action(move |_, _: &A, window, cx| {
        let workspace = cx.entity().downgrade();
        let pages = pages.clone();
        let installed = installed.clone();
        window.defer(cx, move |window, cx| {
            let _ = workspace.update(cx, |workspace, cx| {
                nocterm_settings_ui::open_page(workspace, id, &pages, installed, window, cx)
            });
        });
    });
}

pub(crate) fn register_appearance_commands(
    workspace: &mut Workspace,
    themes: nocterm_ui::ThemeUi,
    settings: nocterm_store::SettingsHandle,
) {
    let picker = themes.clone();
    let theme_settings = settings.clone();
    workspace.register_action(move |_, _: &ChangeTheme, window, cx| {
        let themes = picker.clone();
        let settings = theme_settings.clone();
        window.defer(cx, move |window, cx| {
            nocterm_ui::open_theme_picker(&themes, &settings, window, cx)
        });
    });
    workspace.register_action(move |_, _: &ChangeColorScheme, window, cx| {
        let settings = settings.clone();
        window.defer(cx, move |window, cx| {
            nocterm_ui::open_scheme_picker(&settings, window, cx)
        });
    });
}

#[cfg(test)]
mod tests;
