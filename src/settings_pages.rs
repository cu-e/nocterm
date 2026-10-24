//! App-owned checked settings catalogs and native action routing.
use nocterm_plugin_model::PluginCatalog;
use nocterm_settings_ui::{
    Installed, SettingsCatalog, SettingsCatalogError, SettingsPageRegistration,
};
use nocterm_ui::{SettingsPageSpec, ThemeUi};
use nocterm_workspace::{
    ChangeColorScheme, ChangeTheme, DefaultSessionSettings, OpenAiSettings, OpenKeymap,
    OpenPluginPage, OpenPlugins, OpenSSHSettings, OpenVault, Workspace,
};
use std::sync::Arc;

/// The id of the page that lists the plugins; pages of single plugins nest under it.
const PLUGINS_PAGE: &str = "plugins";

/// The lazy pages after the builtin ones, in navigation order, with the plugin catalog the
/// Plugins page and the plugin page links read.
pub(crate) struct GuestPages {
    pages: Vec<SettingsPageRegistration>,
    plugins: Arc<PluginCatalog>,
}
impl GuestPages {
    pub(crate) fn new(pages: Vec<SettingsPageRegistration>, plugins: Arc<PluginCatalog>) -> Self {
        Self { pages, plugins }
    }
}

#[derive(Clone)]
pub(crate) struct SettingsPages {
    installed: Installed,
    ordinary: SettingsCatalog,
    startup: Option<SettingsCatalog>,
    plugins: Arc<PluginCatalog>,
}
impl SettingsPages {
    /// Gather page registrations from the exact feature installations retained by Booted.
    pub(crate) fn from_installations(
        settings: nocterm_store::SettingsHandle,
        themes: ThemeUi,
        terminal: &nocterm_terminal::Installed,
        agent: &nocterm_agent::Installed,
        sandbox_probe: std::sync::Arc<dyn nocterm_ai::sandbox::SandboxProbe>,
        guests: GuestPages,
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
        guests: GuestPages,
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
        Self::new(installed, builtins, guests, vault)
    }
    pub(crate) fn new(
        installed: Installed,
        builtins: [SettingsPageSpec; 5],
        guests: GuestPages,
        startup_vault: Option<SettingsPageSpec>,
    ) -> Result<Self, SettingsCatalogError> {
        let GuestPages { pages, plugins } = guests;
        let vault = startup_vault.clone().map(SettingsPageRegistration::lazy);
        let ordinary = catalog(
            builtins.clone(),
            pages.into_iter().chain(vault.clone()).collect(),
        )?;
        let startup = vault
            .map(|vault| catalog(builtins, vec![vault]))
            .transpose()?;
        Ok(Self {
            installed,
            ordinary,
            startup,
            plugins,
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
        open_on_anchor::<OpenPlugins>(
            workspace,
            PLUGINS_PAGE,
            |action| action.plugin.clone(),
            &self.ordinary,
            self.installed.clone(),
        );
        self.register_plugin_pages(workspace);
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
    /// The gear of a plugin opens the page the plugin names, enabled or not.
    fn register_plugin_pages(&self, workspace: &mut Workspace) {
        let (plugins, pages, installed) = (
            self.plugins.clone(),
            self.ordinary.clone(),
            self.installed.clone(),
        );
        workspace.register_action(move |_, action: &OpenPluginPage, window, cx| {
            let Some(page) = plugins
                .plugins()
                .find(|plugin| plugin.id.as_str() == action.plugin.as_ref())
                .and_then(|plugin| plugin.settings_page)
            else {
                return;
            };
            defer_open(page, None, &pages, &installed, window, cx);
        });
    }
}
/// Lazy pages are composed from the same feature installations as the window. The page of a
/// single plugin nests under Plugins, anchored at the plugin's id; Plugins comes after the
/// other guests, and the Vault page after it.
pub(crate) fn guest_pages(
    keymap: nocterm_keymap::KeymapHandle,
    monitor: &nocterm_monitor_ui::Installed,
    files: &nocterm_files::Installed,
    projects: Option<&nocterm_projects::Installed>,
    plugins_page: SettingsPageSpec,
    plugins: Arc<PluginCatalog>,
) -> GuestPages {
    let nested = |spec, plugin: &'static nocterm_plugin_model::SystemPlugin| {
        SettingsPageRegistration::lazy(spec).under(PLUGINS_PAGE, plugin.id.as_str())
    };
    let mut pages = vec![
        SettingsPageRegistration::lazy(nocterm_keymap_ui::settings_page(keymap)),
        nested(
            nocterm_monitor_ui::settings_page(monitor),
            &nocterm_monitor_ui::plugin::PLUGIN,
        ),
        SettingsPageRegistration::lazy(nocterm_files::settings_page(files)),
    ];
    pages.extend(projects.map(|projects| {
        nested(
            nocterm_projects::settings_page(projects),
            &nocterm_projects::plugin::PLUGIN,
        )
    }));
    pages.push(SettingsPageRegistration::lazy(plugins_page));
    GuestPages::new(pages, plugins)
}
fn catalog(
    builtins: [SettingsPageSpec; 5],
    guests: Vec<SettingsPageRegistration>,
) -> Result<SettingsCatalog, SettingsCatalogError> {
    SettingsCatalog::new(
        builtins
            .into_iter()
            .map(SettingsPageRegistration::eager)
            .chain(guests)
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
    open_on_anchor::<A>(workspace, id, |_| None, pages, installed);
}
/// Like `open_on`, and asks the page to reveal the anchor the action names.
fn open_on_anchor<A: gpui_kit::Action>(
    workspace: &mut Workspace,
    id: &'static str,
    anchor: fn(&A) -> Option<gpui_kit::SharedString>,
    pages: &SettingsCatalog,
    installed: Installed,
) {
    let pages = pages.clone();
    workspace.register_action(move |_, action: &A, window, cx| {
        let anchor = anchor(action);
        defer_open(id, anchor.as_deref(), &pages, &installed, window, cx);
    });
}
/// Opens the page after the current update, revealing `anchor` when it is given.
fn defer_open(
    id: &'static str,
    anchor: Option<&str>,
    pages: &SettingsCatalog,
    installed: &Installed,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<Workspace>,
) {
    let workspace = cx.entity().downgrade();
    let (pages, installed) = (pages.clone(), installed.clone());
    let anchor = anchor.map(str::to_owned);
    window.defer(cx, move |window, cx| {
        let _ = workspace.update(cx, |workspace, cx| {
            nocterm_settings_ui::open_page_at(
                workspace,
                id,
                anchor.as_deref(),
                &pages,
                installed,
                window,
                cx,
            )
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
