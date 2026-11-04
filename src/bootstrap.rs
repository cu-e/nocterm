//! Composition order for services and explicit feature dependencies, shared by
//! application and GUI fixtures. Booted retains one terminal installation for all
//! windows; authentication captures it after settings and credentials are installed.
//! Successful snippet installation retains the exact library/settings across windows.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _};
use nocterm_core::{Paths, clock::WallClock};
use nocterm_design::DesignTokens;
use nocterm_session::Transport;
use nocterm_store::SettingsConfig;
use nocterm_vault::VaultAccess;
use std::rc::Rc;

use crate::shell_setup::{GuestInputs, Plugins, UserPluginMode, install_guest_pages};
use crate::window_sources::{WindowSources, WindowSourcesError};
use nocterm_hubs_model::Environment;

mod error;
mod vault;
mod workspace;
pub(crate) use error::BootstrapError;
pub(crate) use vault::credential_provider;
use vault::{install_vault, open_vault};
pub(crate) use workspace::build_workspace;

/// What differs between the application and its tests.
pub(crate) struct Services {
    pub(crate) tokens: DesignTokens,
    pub(crate) settings: SettingsConfig,
    pub(crate) paths: Paths,
    /// Whether connections, snippets, layout, recordings and the keymap are
    /// read from and written to `paths`; tests keep them in memory.
    pub(crate) persist: bool,
    pub(crate) hosts: HostServices,
    pub(crate) terminal_auth_factory:
        Option<fn(nocterm_terminal::Installed) -> nocterm_agent::TerminalAuthOpener>,
    pub(crate) files: nocterm_files::Deps,
    pub(crate) themes: Option<Themes>,
    pub(crate) agent: nocterm_agent::AgentServices,
    /// Whether agents can be sandboxed here, shown on the AI settings page.
    pub(crate) sandbox_probe: Arc<dyn nocterm_ai::sandbox::SandboxProbe>,
    pub(crate) images: nocterm_agent::ImageDeps,
    pub(crate) vault: Option<VaultSetup>,
    pub(crate) connections: nocterm_connections::PreparedConnections,
    pub(crate) projects: crate::project_setup::PreparedProjectSetup,
    /// Which user plugins this run opens. The services are at the field limit: a new way to
    /// start extends this enum, not the struct.
    pub(crate) user_plugins: UserPluginMode,
}

/// Native connection capabilities supplied once for terminal composition.
pub(crate) struct HostServices {
    pub(crate) transport: Arc<dyn Transport>,
    pub(crate) local: nocterm_terminal::LocalSessions,
    pub(crate) local_host: Option<Arc<dyn nocterm_session::HostExec>>,
    pub(crate) image_search: Arc<dyn nocterm_project_providers::ImageSearch>,
    pub(crate) environment: Arc<dyn nocterm_project_providers::WorkspaceEnvironment>,
    pub(crate) preparation: Arc<dyn nocterm_project_providers::ProjectPreparation>,
    pub(crate) container_sessions: Arc<dyn nocterm_project_providers::ContainerSessionFactory>,
    pub(crate) workspace_sessions: Arc<dyn nocterm_project_providers::WorkspaceSessionFactory>,
    pub(crate) container_actions: Arc<dyn nocterm_project_providers::WorkspaceContainerActions>,
    /// The hub access ports; `None` leaves the application without hubs.
    pub(crate) hubs: Option<nocterm_hubs::Deps>,
    /// The Cloud agents ports; without them the new-chat menu has no Cloud tab.
    pub(crate) cloud_agents: Option<nocterm_cloud_agents::Deps>,
    /// Creates, lists and opens Cloud chats; without it the Cloud tab offers agents only.
    pub(crate) cloud_chats: Option<Arc<dyn nocterm_cloud_agents_model::CloudChatLauncher>>,
    /// The encrypted data of hub accounts; without it the hubs cannot open windows.
    pub(crate) hub_accounts: Option<Arc<dyn crate::hub_sources::HubAccounts>>,
    /// Receives the sessions of saved connections for the agent SSH CA; `None` without hubs.
    pub(crate) agent_hosts: Option<Arc<dyn nocterm_ssh_keys::authorized_keys::ConnectedHosts>>,
}

/// The hub servers every window shows, and the sources their windows are made from.
#[derive(Clone)]
pub(crate) struct HubWindows {
    ui: nocterm_hubs_ui::Installed,
    /// Without access to hub accounts the servers can be listed but no window shows one.
    sources: Option<gpui_kit::Entity<crate::hub_sources::HubSources>>,
}

/// The hub servers with the account data their windows will be made from.
pub(crate) struct PendingHubs {
    pub(crate) hubs: nocterm_hubs::Installed,
    accounts: Option<Arc<dyn crate::hub_sources::HubAccounts>>,
    /// The Cloud agents the Servers page manages personal agents of.
    cloud: Option<gpui_kit::Entity<nocterm_cloud_agents::CloudAgents>>,
    /// Receives the sessions of saved connections for the agent SSH CA.
    agent_hosts: Option<Arc<dyn nocterm_ssh_keys::authorized_keys::ConnectedHosts>>,
}

/// Lets the agent SSH CA follow the saved connections once both are installed.
fn observe_agent_hosts(
    pending: Option<&PendingHubs>,
    connections: Option<&nocterm_connections::Installed>,
) {
    if let (Some(pending), Some(connections)) = (pending, connections)
        && let Some(sessions) = pending.agent_hosts.clone()
    {
        crate::agent_hosts::observe(sessions, pending.hubs.hubs().clone(), connections);
    }
}

fn start_hubs(
    pending: Option<PendingHubs>,
    inputs: nocterm_connections::ConnectionInputs,
    terminal: &nocterm_terminal::Installed,
    ready: &nocterm_ui::UiReady,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> Option<HubWindows> {
    let PendingHubs {
        hubs,
        accounts,
        cloud,
        ..
    } = pending?;
    let sources = accounts.map(|accounts| {
        let (hubs, terminal) = (hubs.hubs().clone(), terminal.clone());
        let (ready, shutdown) = (ready.clone(), shutdown.clone());
        cx.new(|_| {
            crate::hub_sources::HubSources::new(accounts, hubs, inputs, terminal, ready, shutdown)
        })
    });
    let ui = nocterm_hubs_ui::install(hubs);
    Some(HubWindows {
        ui: match cloud {
            Some(cloud) => ui.with_cloud_agents(cloud),
            None => ui,
        },
        sources,
    })
}

/// The connection editor offers the keys of the vault.
fn offer_keys(
    connections: nocterm_connections::PreparedConnections,
    vault: Option<&Arc<dyn VaultAccess>>,
    cx: &App,
) -> nocterm_connections::PreparedConnections {
    let Some(vault) = vault else {
        return connections;
    };
    connections.with_keys(crate::vault_keys::keys_of(vault, cx))
}

struct ProjectResources {
    settings: nocterm_store::SettingsHandle,
    directories: Arc<dyn nocterm_explorer::directory::DirectoryReaders>,
    image_search: Arc<dyn nocterm_project_providers::ImageSearch>,
    environment: Arc<dyn nocterm_project_providers::WorkspaceEnvironment>,
    preparation: Arc<dyn nocterm_project_providers::ProjectPreparation>,
    container_actions: Arc<dyn nocterm_project_providers::WorkspaceContainerActions>,
    local_host: Option<Arc<dyn nocterm_session::HostExec>>,
}

impl ProjectResources {
    fn new(
        hosts: &HostServices,
        settings: nocterm_store::SettingsHandle,
        directories: Arc<dyn nocterm_explorer::directory::DirectoryReaders>,
    ) -> Self {
        Self {
            settings,
            directories,
            image_search: hosts.image_search.clone(),
            environment: hosts.environment.clone(),
            preparation: hosts.preparation.clone(),
            container_actions: hosts.container_actions.clone(),
            local_host: hosts.local_host.clone(),
        }
    }
}

pub(crate) struct Themes {
    pub(crate) library: Arc<dyn nocterm_themes::ThemeLibrary>,
    pub(crate) catalog: nocterm_themes::ThemeCatalog,
    pub(crate) registry: Arc<dyn nocterm_themes::ThemeRegistry>,
}

pub(crate) struct VaultSetup {
    pub(crate) file: std::path::PathBuf,
    /// Whether the platform's authenticated unlock is offered.
    pub(crate) device_unlock: bool,
}

/// Installed dependencies retained by every workspace window.
#[derive(Clone)]
pub(crate) struct Booted {
    #[cfg(test)]
    keymap: nocterm_keymap::KeymapHandle,
    /// This computer's executor and the one watch of each host's containers.
    containers: nocterm_containers_ui::Installed,
    terminal: nocterm_terminal::Installed,
    settings_ui: crate::settings_pages::SettingsPages,
    monitor: nocterm_monitor_ui::Installed,
    agent: nocterm_agent::Installed,
    files: nocterm_files::Installed,
    workspace: nocterm_workspace::Deps,
    connections: Option<nocterm_connections::Installed>,
    snippets: Option<nocterm_snippets_ui::Installed>,
    projects: Option<nocterm_projects::Installed>,
    /// The hub servers, shared by every window.
    hubs: Option<HubWindows>,
    themes: nocterm_ui::ThemeUi,
    user_plugins: crate::shell_setup::UserShell,
    /// The vault, when it opened; credentials and its settings depend on it.
    vault: Option<Rc<nocterm_vault_ui::Installed>>,
}

impl Booted {
    pub(crate) fn themes(&self) -> &nocterm_ui::ThemeUi {
        &self.themes
    }
    #[cfg(test)]
    pub(crate) fn vault(&self) -> Option<&Rc<nocterm_vault_ui::Installed>> {
        self.vault.as_ref()
    }

    /// The sources of a window that shows `environment`: this computer's connections, vault and
    /// terminal capability, or the account of a hub.
    pub(crate) fn window_sources(
        &self,
        environment: &Environment,
        cx: &mut App,
    ) -> Result<WindowSources, WindowSourcesError> {
        let Environment::Hub(hub) = environment else {
            return Ok(self.local_sources());
        };
        let sources = self
            .hubs
            .as_ref()
            .and_then(|hubs| hubs.sources.clone())
            .ok_or(WindowSourcesError::NoHubs)?;
        let source = sources.update(cx, |sources, cx| {
            sources.source(hub, self.agent.runtime().clone(), cx)
        })?;
        let settings = self
            .settings_ui
            .with_vault(nocterm_vault_ui::settings_page(source.vault()))?;
        Ok(WindowSources::new(
            environment.clone(),
            Some(source.connections().clone()),
            source.terminal().clone(),
            Some(source.vault().clone()),
            settings,
        )
        .with_agent_source(source.agent_source()))
    }

    /// The saved workspaces as a window of `sources` opens them: with the terminals and the
    /// saved connections of that source instead of the ones of this computer.
    pub(crate) fn projects_of(
        &self,
        sources: &WindowSources,
    ) -> Option<nocterm_projects::Installed> {
        let projects = self.projects.clone()?;
        let resolver = match crate::project_setup::terminal_resolver(
            sources.terminal(),
            sources.connections(),
        ) {
            Ok(resolver) => resolver,
            Err(error) => {
                tracing::error!(%error, "workspace terminals of this window are unavailable");
                return None;
            }
        };
        Some(
            projects
                .with_terminals(nocterm_terminal::project_terminal_opener(
                    sources.terminal(),
                ))
                .with_terminal_resolver(resolver),
        )
    }

    /// The sources of a window that shows this computer.
    pub(crate) fn local_sources(&self) -> WindowSources {
        WindowSources::new(
            Environment::Local,
            self.connections.clone(),
            self.terminal.clone(),
            self.vault.clone(),
            self.settings_ui.clone(),
        )
    }

    #[cfg(test)]
    pub(crate) fn connections(&self) -> Option<&nocterm_connections::Installed> {
        self.connections.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn keymap(&self) -> nocterm_keymap::KeymapHandle {
        self.keymap.clone()
    }
    #[cfg(all(test, feature = "user-plugins"))]
    pub(crate) fn user_plugins(&self) -> &crate::shell_setup::UserShell {
        &self.user_plugins
    }
    #[cfg(test)]
    pub(crate) fn agent_installation(&self) -> &nocterm_agent::Installed {
        &self.agent
    }
    pub(crate) fn settings_deps(&self) -> nocterm_settings_ui::Installed {
        self.settings_ui.installed()
    }
    #[cfg(test)]
    pub(crate) fn settings_pages(&self) -> nocterm_settings_ui::SettingsCatalog {
        self.settings_ui.ordinary().clone()
    }

    pub(crate) fn hubs(&self) -> Option<&nocterm_hubs_ui::Installed> {
        self.hubs.as_ref().map(|hubs| &hubs.ui)
    }

    #[cfg(test)]
    pub(crate) fn hub_registry(&self, cx: &App) -> Option<gpui_kit::Entity<nocterm_hubs::Hubs>> {
        let sources = self.hubs.as_ref()?.sources.as_ref()?;
        Some(sources.read(cx).registry().clone())
    }

    pub(crate) fn workspace_deps(&self) -> nocterm_workspace::Deps {
        self.workspace.clone()
    }
}

/// Composes services and feature dependencies in dependency order.
pub(crate) fn bootstrap(services: Services, cx: &mut App) -> Result<Booted, BootstrapError> {
    let Services {
        tokens,
        settings,
        paths,
        persist,
        hosts,
        terminal_auth_factory,
        files,
        themes,
        agent,
        sandbox_probe,
        images,
        vault,
        connections,
        projects,
        user_plugins: mode,
    } = services;
    let stored = persist.then_some(&paths);
    let (ui, shutdown, themes, layout) = install_interface(tokens, settings, themes, stored, cx);
    let resources = ProjectResources::new(&hosts, ui.settings().clone(), files.directory_readers());
    let local_host = hosts.local_host.clone();
    let (hubs, cloud_agents) = install_hubs(&hosts, &ui, cx);
    let terminal_deps = terminal_dependencies(hosts, stored, &shutdown);
    // The vault opens first: the connection editor offers its keys, and its
    // credentials need the installed connections afterwards.
    let vault = vault.and_then(|setup| open_vault(setup, &ui, cx));
    let connections = offer_keys(connections, vault.as_ref(), cx);
    let connection_inputs = connections.inputs();
    let connections = install_connections(connections, &ui, &shutdown, cx);
    observe_agent_hosts(hubs.as_ref(), connections.as_ref());
    let (terminal, vault) = install_terminal(terminal_deps, vault, &ui, connections.as_ref(), cx);
    let hubs = start_hubs(hubs, connection_inputs, &terminal, &ui, &shutdown, cx);
    let projects = install_projects(
        projects,
        &terminal,
        connections.as_ref(),
        resources,
        &shutdown,
        cx,
    );
    let terminal_auth = terminal_auth_factory.map(|factory| factory(terminal.clone()));
    let snippets = install_snippets(stored, ui.settings(), &shutdown, cx);
    let agent = install_agent(agent, terminal_auth, images, &layout, &ui, &shutdown, cx);
    let agent = agent.with_cloud_agents(cloud_agents);
    let files = install_files(files, &ui, &shutdown, cx);
    let keymap = install_keymap(stored, cx);
    let clock: Arc<dyn WallClock> = Arc::new(nocterm_local::SystemClock);
    let plugins = Plugins::install(&paths, &shutdown, clock.clone(), mode, ui.settings(), cx)?;
    let monitor = nocterm_monitor_ui::install(local_host.clone(), &ui, cx);
    let inputs = GuestInputs::new(&monitor, &ui, keymap.clone(), &files, &layout)
        .with_projects(projects.as_ref())
        .with_hubs(hubs.as_ref().map(|hubs| &hubs.ui));
    let guests = install_guest_pages(inputs, &plugins, cx);
    let settings_ui = crate::settings_pages::SettingsPages::from_installations(
        ui.settings().clone(),
        themes.clone(),
        &terminal,
        &agent,
        sandbox_probe,
        guests,
        vault.as_deref(),
    )?;
    let booted = Booted {
        #[cfg(test)]
        keymap,
        containers: nocterm_containers_ui::install(local_host),
        terminal,
        settings_ui,
        monitor,
        agent,
        connections,
        snippets,
        projects,
        hubs,
        vault,
        files,
        user_plugins: plugins.user().clone(),
        workspace: workspace_deps(layout, &plugins, ui.settings().clone(), clock),
        themes,
    };
    crate::application::register(paths, booted.clone(), ui.settings().clone(), cx);
    Ok(booted)
}

/// The shared dependencies every window's workspace is made from.
fn workspace_deps(
    layout: nocterm_ui::LayoutHandle,
    plugins: &Plugins,
    settings: nocterm_store::SettingsHandle,
    clock: Arc<dyn WallClock>,
) -> nocterm_workspace::Deps {
    nocterm_workspace::Deps::new(layout)
        .with_plugin_source(plugins.source())
        .with_settings(settings)
        .with_clock(clock)
}

/// Establish shared settings, appearance and durable layout before feature installation.
fn install_interface(
    tokens: DesignTokens,
    settings: SettingsConfig,
    themes: Option<Themes>,
    stored: Option<&Paths>,
    cx: &mut App,
) -> (
    nocterm_ui::UiReady,
    nocterm_store::ShutdownCoordinator,
    nocterm_ui::ThemeUi,
    nocterm_ui::LayoutHandle,
) {
    gpui_kit::init(cx);
    let base = tokens.clone();
    let settings = settings.install(cx);
    let ui = nocterm_ui::init(tokens, settings, cx);
    let shutdown = nocterm_store::ShutdownCoordinator::default();
    report_store_registration("settings", ui.settings().register_shutdown(&shutdown));
    let themes = install_themes(themes, ui.settings().clone(), base, cx);
    let layout = nocterm_ui::LayoutMemory::init(
        stored.map(|paths| paths.state_dir().join("layout.json")),
        &shutdown,
        cx,
    );
    (ui, shutdown, themes, layout)
}

/// Saved workspaces retain the original terminal installation and shutdown owner.
fn install_projects(
    projects: crate::project_setup::PreparedProjectSetup,
    terminal: &nocterm_terminal::Installed,
    connections: Option<&nocterm_connections::Installed>,
    resources: ProjectResources,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> Option<nocterm_projects::Installed> {
    match projects.install(terminal, connections, resources.settings, shutdown, cx) {
        Ok(projects) => Some(
            projects
                .with_directory_readers(resources.directories)
                .with_preparation(resources.preparation)
                .with_image_search(resources.image_search)
                .with_environment(resources.environment)
                .with_container_actions(resources.container_actions)
                .with_local_host(resources.local_host),
        ),
        Err(error) => {
            tracing::error!(%error, "workspace installation failed");
            None
        }
    }
}

/// Saved connections, or none when their store could not register for shutdown.
fn install_connections(
    deps: nocterm_connections::PreparedConnections,
    ui: &nocterm_ui::UiReady,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> Option<nocterm_connections::Installed> {
    match nocterm_connections::install(deps, ui, shutdown, cx) {
        Ok(connections) => Some(connections),
        Err(error) => {
            report_store_registration("connections", Err(error));
            None
        }
    }
}

/// Bind credentials before installing the retained terminal capability.
fn install_terminal(
    terminal: nocterm_terminal::Deps,
    vault: Option<Arc<dyn VaultAccess>>,
    ready: &nocterm_ui::UiReady,
    connections: Option<&nocterm_connections::Installed>,
    cx: &mut App,
) -> (
    nocterm_terminal::Installed,
    Option<Rc<nocterm_vault_ui::Installed>>,
) {
    let association_owner = connections.map(|installed| installed.connections().clone());
    let spaces = connections
        .map(nocterm_connections::Installed::credential_spaces)
        .unwrap_or_default();
    let vault = vault.map(|vault| install_vault(vault, ready, association_owner, spaces, cx));
    let (terminal, vault) = match vault {
        Some(opened) => (
            terminal.with_credentials(opened.credentials),
            Some(opened.installed),
        ),
        None => (terminal, None),
    };
    (nocterm_terminal::install(terminal, ready, cx), vault)
}

/// Compose the supplied theme catalogue and its retained settings/paint projection.
fn install_themes(
    themes: Option<Themes>,
    settings: nocterm_store::SettingsHandle,
    base: DesignTokens,
    cx: &mut App,
) -> nocterm_ui::ThemeUi {
    let (library, registry, catalog) = match themes {
        Some(themes) => (Some(themes.library), Some(themes.registry), themes.catalog),
        None => (None, None, nocterm_themes::ThemeCatalog::default()),
    };
    let themes = nocterm_theme_service::ThemesHandle::install(library, registry, catalog, cx);
    nocterm_ui::ThemeUi::install(settings, themes, base, cx)
}

/// The hub servers, when the application has hub access.
fn install_hubs(
    hosts: &HostServices,
    ui: &nocterm_ui::UiReady,
    cx: &mut App,
) -> (
    Option<PendingHubs>,
    Option<nocterm_agent::CloudAgentsAccess>,
) {
    let Some(deps) = hosts.hubs.clone() else {
        return (None, None);
    };
    let mut pending = PendingHubs {
        hubs: nocterm_hubs::install(deps, ui.settings().clone(), cx),
        accounts: hosts.hub_accounts.clone(),
        cloud: None,
        agent_hosts: hosts.agent_hosts.clone(),
    };
    let cloud = crate::cloud_agents_wiring::install(
        hosts.cloud_agents.clone(),
        hosts.cloud_chats.clone(),
        Some(&pending),
        cx,
    );
    pending.cloud = cloud.as_ref().map(|access| access.service().clone());
    (Some(pending), cloud)
}

/// The shared keymap: the defaults and the user's overrides, when the application keeps files.
fn install_keymap(stored: Option<&Paths>, cx: &mut App) -> nocterm_keymap::KeymapHandle {
    let user_keymap = stored.map(|paths| paths.config_dir().join("keymap.toml"));
    crate::keymap::load(user_keymap, cx)
}

/// Retain only installations registered with the common shutdown coordinator.
fn install_snippets(
    paths: Option<&nocterm_core::Paths>,
    settings: &nocterm_store::SettingsHandle,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> Option<nocterm_snippets_ui::Installed> {
    use nocterm_snippets::SnippetRepository as _;
    let repository = paths.map(|paths| {
        Arc::new(nocterm_snippets_io::FileSnippetRepository::new(
            paths.snippets_file(),
        ))
    });
    let loaded = repository
        .as_ref()
        .map_or(Ok(None), |repository| repository.load());
    let prepared = nocterm_snippets_store::PreparedLibrary::loaded(loaded, repository.is_some());
    let writer: nocterm_store::SnapshotWriter<nocterm_snippets::Library> =
        Arc::new(move |library| {
            let repository = repository.clone();
            Box::pin(async move {
                if let Some(repository) = repository {
                    repository
                        .save(&library)
                        .map_err(nocterm_store::StoreError::backend)?;
                }
                Ok(())
            })
        });
    match nocterm_snippets_store::SnippetsHandle::install(prepared, writer, shutdown, cx) {
        Ok(model) => Some(nocterm_snippets_ui::init(
            model,
            settings.clone(),
            Arc::new(nocterm_snippets_io::NativeSnippetIds),
            cx,
        )),
        Err(error) => {
            report_store_registration("snippets", Err(error));
            None
        }
    }
}

/// Compose one registry and native session capabilities for all windows.
fn terminal_dependencies(
    hosts: HostServices,
    stored: Option<&nocterm_core::Paths>,
    shutdown: &nocterm_store::ShutdownCoordinator,
) -> nocterm_terminal::Deps {
    let deps = nocterm_terminal::Deps::new(hosts.transport, hosts.local)
        .with_container_sessions(hosts.container_sessions)
        .with_workspace_sessions(hosts.workspace_sessions);
    let Some(paths) = stored else {
        return deps;
    };
    let log_factory = Arc::new(nocterm_local::LocalOutputLogFactory::new(
        paths.state_dir().join("logs"),
    ));
    let registry = Arc::new(nocterm_store::RecordingRegistry::new(log_factory));
    report_store_registration("recording", registry.register(shutdown));
    deps.with_recording(registry)
}

/// Installs the agent feature; a supplied terminal sign-in opener replaces the one in `agent`.
fn install_agent(
    mut agent: nocterm_agent::AgentServices,
    terminal_auth: Option<nocterm_agent::TerminalAuthOpener>,
    images: nocterm_agent::ImageDeps,
    layout: &nocterm_ui::LayoutHandle,
    ui: &nocterm_ui::UiReady,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> nocterm_agent::Installed {
    if terminal_auth.is_some() {
        agent.terminal_auth = terminal_auth;
    }
    let (agent, registration) =
        nocterm_agent::init(agent, images, layout.clone(), ui, shutdown, cx);
    report_store_registration("agent", registration);
    agent
}

/// Freeze transfer admission during synchronous capture, then drain without App.
/// Files installation and its transfer shutdown participant share one lifetime.
fn install_files(
    deps: nocterm_files::Deps,
    ui: &nocterm_ui::UiReady,
    shutdown: &nocterm_store::ShutdownCoordinator,
    cx: &mut App,
) -> nocterm_files::Installed {
    let files = nocterm_files::install(deps, ui, cx);
    register_transfer_shutdown(&files, shutdown);
    files
}

fn register_transfer_shutdown(
    files: &nocterm_files::Installed,
    shutdown: &nocterm_store::ShutdownCoordinator,
) {
    let files = files.clone();
    report_store_registration(
        "transfers",
        shutdown.register_capture(move |_| {
            let drain = files.freeze_transfers();
            Box::pin(async move { drain.await.map_err(nocterm_store::StoreError::backend) })
        }),
    );
}

/// A feature the workspace refused to place stays installed without its panel or widget.
fn report_registration(owner: &str, result: Result<(), nocterm_workspace::RegistrationError>) {
    if let Err(error) = result {
        tracing::error!(owner, %error, "shell registration failed");
    }
}

/// Reports registration failures consistently while retaining installation order.
fn report_store_registration(owner: &str, result: Result<(), nocterm_store::StoreError>) {
    if let Err(error) = result {
        tracing::error!(owner, %error, "shutdown registration failed");
    }
}

#[cfg(test)]
mod credential_tests;
#[cfg(test)]
mod observation_tests;
