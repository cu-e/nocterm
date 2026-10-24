//! Composition order for services and explicit feature dependencies, shared by
//! application and GUI fixtures. Booted retains one terminal installation for all
//! windows; authentication captures it after settings and credentials are installed.
//! Successful snippet installation retains the exact library/settings across windows.

use std::sync::Arc;

use gpui_kit::{App, Context, Window};
use nocterm_core::Paths;
use nocterm_design::DesignTokens;
use nocterm_session::Transport;
use nocterm_store::SettingsConfig;
use nocterm_vault::VaultAccess;
use nocterm_vault_store::VaultService;
use nocterm_workspace::Workspace;
use std::rc::Rc;

mod error;
pub(crate) use error::BootstrapError;

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
}

struct ProjectResources {
    settings: nocterm_store::SettingsHandle,
    directories: Arc<dyn nocterm_explorer::directory::DirectoryReaders>,
    image_search: Arc<dyn nocterm_project_providers::ImageSearch>,
    environment: Arc<dyn nocterm_project_providers::WorkspaceEnvironment>,
    preparation: Arc<dyn nocterm_project_providers::ProjectPreparation>,
    container_actions: Arc<dyn nocterm_project_providers::WorkspaceContainerActions>,
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
    local_host: Option<Arc<dyn nocterm_session::HostExec>>,
    terminal: nocterm_terminal::Installed,
    settings_ui: crate::settings_pages::SettingsPages,
    monitor: nocterm_monitor_ui::Installed,
    agent: nocterm_agent::Installed,
    files: nocterm_files::Installed,
    workspace: nocterm_workspace::Deps,
    connections: Option<nocterm_connections::Installed>,
    snippets: Option<nocterm_snippets_ui::Installed>,
    projects: Option<nocterm_projects::Installed>,
    themes: nocterm_ui::ThemeUi,
    /// The vault, when it opened; credentials and its settings depend on it.
    vault: Option<Rc<nocterm_vault_ui::Installed>>,
}

impl Booted {
    pub(crate) fn themes(&self) -> &nocterm_ui::ThemeUi {
        &self.themes
    }
    pub(crate) fn vault(&self) -> Option<&Rc<nocterm_vault_ui::Installed>> {
        self.vault.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn keymap(&self) -> nocterm_keymap::KeymapHandle {
        self.keymap.clone()
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
        mut agent,
        sandbox_probe,
        images,
        vault,
        connections,
        projects,
    } = services;
    let stored = persist.then_some(&paths);
    let (ui, shutdown, themes, layout) = install_interface(tokens, settings, themes, stored, cx);
    let resources = ProjectResources::new(&hosts, ui.settings().clone(), files.directory_readers());
    let local_host = hosts.local_host;
    let terminal_deps = terminal_dependencies(
        hosts.transport,
        hosts.local,
        hosts.container_sessions,
        hosts.workspace_sessions,
        stored,
        &shutdown,
    );
    let connections = install_connections(connections, &ui, &shutdown, cx);
    let (terminal, vault) = install_terminal(terminal_deps, vault, &ui, connections.as_ref(), cx);
    let projects = install_projects(
        projects,
        &terminal,
        connections.as_ref(),
        resources,
        &shutdown,
        cx,
    )
    .map(|projects| projects.with_local_host(local_host.clone()));
    if let Some(factory) = terminal_auth_factory {
        agent.terminal_auth = Some(factory(terminal.clone()));
    }
    let snippets = install_snippets(stored, ui.settings(), &shutdown, cx);
    let (agent, registration) =
        nocterm_agent::init(agent, images, layout.clone(), &ui, &shutdown, cx);
    report_store_registration("agent", registration);
    let files = install_files(files, &ui, &shutdown, cx);
    let user_keymap = stored.map(|paths| paths.config_dir().join("keymap.toml"));
    let keymap = crate::keymap::load(user_keymap, cx);
    let monitor = nocterm_monitor_ui::install(local_host.clone(), &ui, cx);
    let settings_ui = crate::settings_pages::SettingsPages::from_installations(
        ui.settings().clone(),
        themes.clone(),
        &terminal,
        &agent,
        sandbox_probe,
        crate::settings_pages::guest_pages(keymap.clone(), &monitor, &files, projects.as_ref()),
        vault.as_deref(),
    )?;
    let plugins = crate::shell_setup::plugin_catalog()?;
    let booted = Booted {
        #[cfg(test)]
        keymap,
        local_host,
        terminal,
        settings_ui,
        monitor,
        agent,
        connections,
        snippets,
        projects,
        vault,
        files,
        workspace: nocterm_workspace::Deps::new(layout).with_catalog(plugins),
        themes,
    };
    crate::application::register(paths, booted.clone(), ui.settings().clone(), cx);
    Ok(booted)
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
                .with_container_actions(resources.container_actions),
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
    vault: Option<VaultSetup>,
    ready: &nocterm_ui::UiReady,
    connections: Option<&nocterm_connections::Installed>,
    cx: &mut App,
) -> (
    nocterm_terminal::Installed,
    Option<Rc<nocterm_vault_ui::Installed>>,
) {
    let association_owner = connections.map(|installed| installed.connections().clone());
    let vault = vault.and_then(|setup| open_vault(setup, ready, association_owner, cx));
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
    transport: Arc<dyn nocterm_session::Transport>,
    local: nocterm_terminal::LocalSessions,
    container_sessions: Arc<dyn nocterm_project_providers::ContainerSessionFactory>,
    workspace_sessions: Arc<dyn nocterm_project_providers::WorkspaceSessionFactory>,
    stored: Option<&nocterm_core::Paths>,
    shutdown: &nocterm_store::ShutdownCoordinator,
) -> nocterm_terminal::Deps {
    let deps = nocterm_terminal::Deps::new(transport, local)
        .with_container_sessions(container_sessions)
        .with_workspace_sessions(workspace_sessions);
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

struct OpenedVault {
    installed: Rc<nocterm_vault_ui::Installed>,
    credentials: nocterm_terminal::CredentialProvider,
}

/// Opens the vault and lets terminals keep credentials in it.
fn open_vault(
    setup: VaultSetup,
    ready: &nocterm_ui::UiReady,
    connections: Option<gpui_kit::Entity<nocterm_connections::Connections>>,
    cx: &mut App,
) -> Option<OpenedVault> {
    let device = setup.device_unlock.then(nocterm_device_unlock::provider);
    ready
        .settings()
        .register::<nocterm_vault_ui::VaultSettings>(cx);
    let duration = std::time::Duration::from_secs(
        u64::from(
            ready
                .settings()
                .setting::<nocterm_vault_ui::VaultSettings>(cx)
                .auto_lock_minutes,
        ) * 60,
    );
    let vault: Arc<dyn VaultAccess> =
        match VaultService::new_with_device_unlock(setup.file, duration, device) {
            Ok(vault) => Arc::new(vault),
            Err(error) => {
                tracing::error!(%error, "could not initialize the credential vault");
                return None;
            }
        };
    let credentials = nocterm_terminal::CredentialProvider::new(
        Arc::new(nocterm_vault_ui::VaultCredentials::new(vault.clone())),
        move |spec, id, cx| {
            let Some(connections) = connections.as_ref() else {
                return gpui_kit::Task::ready(Err(
                    nocterm_terminal::CredentialAssociationError::Unavailable,
                ));
            };
            // A secret remembered for a machine behind jump hosts binds only to
            // the connection it was typed for, never to one that merely shares its address.
            let Some(scope) =
                nocterm_connections_model::credential_scope::CredentialScope::of_session(
                    spec.target.clone(),
                    spec.auth.clone(),
                    spec.profile
                        .as_deref()
                        .and_then(|profile| profile.parse().ok()),
                    !spec.jumps.is_empty(),
                )
            else {
                return gpui_kit::Task::ready(Ok(()));
            };
            let association = connections.update(cx, |connections, cx| {
                connections.associate_credential(scope, id, cx)
            });
            cx.spawn(async move |_| {
                association
                    .await
                    .map_err(nocterm_terminal::CredentialAssociationError::binding)
            })
        },
    );
    Some(OpenedVault {
        installed: Rc::new(nocterm_vault_ui::install(
            nocterm_vault_ui::Deps::new(vault),
            ready,
            cx,
        )),
        credentials,
    })
}

/// Plugs every feature into a new window's workspace.
pub(crate) fn build_workspace(
    workspace: &mut Workspace,
    booted: &Booted,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let vault = booted.vault().map(Rc::as_ref);
    nocterm_terminal::register(&booted.terminal, workspace);
    if let Some(connections) = &booted.connections {
        nocterm_connections::register(connections, workspace, window, cx);
        workspace.set_chat_connection_opener(crate::project_setup::chat_connection_opener(
            &booted.terminal,
            connections,
        ));
    }
    booted
        .settings_ui
        .register(workspace, booted.themes().clone());
    nocterm_files::register(&booted.files, workspace, window, cx);
    if let Some(snippets) = &booted.snippets {
        report_registration(
            "snippets",
            nocterm_snippets_ui::register(workspace, snippets, window, cx),
        );
    }
    nocterm_containers_ui::register(booted.local_host.clone(), workspace, window, cx);
    if let Some(projects) = &booted.projects {
        nocterm_projects::register(workspace, projects, window, cx);
    }
    nocterm_monitor_ui::register(&booted.monitor, workspace, window, cx);
    nocterm_agent::register(workspace, booted.agent.clone(), window, cx);
    workspace.set_menu_builder(crate::app_menus::build, window, cx);
    if let Some(vault) = vault
        && vault
            .settings()
            .setting::<nocterm_vault_ui::VaultSettings>(cx)
            .prompt_on_startup
        && let Some(pages) = booted.settings_ui.startup()
    {
        nocterm_settings_ui::open_page(
            workspace,
            "vault",
            pages,
            booted.settings_deps(),
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod credential_tests;
#[cfg(test)]
mod observation_tests;
