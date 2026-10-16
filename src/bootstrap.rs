//! Composition order for services and explicit feature dependencies, shared by
//! application and GUI fixtures. Booted retains one terminal installation for all
//! windows; authentication captures it after settings and credentials are installed.
//! Successful snippet installation retains the exact library/settings across windows.

use std::sync::Arc;

use gpui_kit::{App, Context, Window};
use nocterm_core::Paths;
use nocterm_design::DesignTokens;
use nocterm_session::Transport;
use nocterm_ui::SettingsStore;
use nocterm_vault::VaultAccess;
use nocterm_vault_store::VaultService;
use nocterm_workspace::Workspace;
use std::rc::Rc;

/// What differs between the application and its tests.
pub(crate) struct Services {
    pub(crate) tokens: DesignTokens,
    pub(crate) settings: SettingsStore,
    pub(crate) paths: Paths,
    /// Whether connections, snippets, layout, recordings and the keymap are
    /// read from and written to `paths`; tests keep them in memory.
    pub(crate) persist: bool,
    pub(crate) transport: Arc<dyn Transport>,
    /// Whether local shells run on this machine.
    pub(crate) local: nocterm_terminal::LocalSessions,
    /// Local host execution, supplied once independently of session creation.
    pub(crate) local_host: Option<Arc<dyn nocterm_session::HostExec>>,
    pub(crate) terminal_auth_factory:
        Option<fn(nocterm_terminal::Installed) -> nocterm_agent::TerminalAuthOpener>,
    pub(crate) files: nocterm_files::Deps,
    pub(crate) themes: Option<Themes>,
    pub(crate) agent: nocterm_agent::AgentServices,
    pub(crate) images: nocterm_agent::ImageDeps,
    pub(crate) vault: Option<VaultSetup>,
    pub(crate) connections: nocterm_connections::PreparedConnections,
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
    keymap: nocterm_keymap::KeymapHandle,
    local_host: Option<Arc<dyn nocterm_session::HostExec>>,
    terminal: nocterm_terminal::Installed,
    settings_ui: nocterm_settings_ui::Installed,
    monitor: nocterm_monitor_ui::Installed,
    agent: nocterm_agent::Installed,
    files: nocterm_files::Installed,
    workspace: nocterm_workspace::Deps,
    connections: Option<nocterm_connections::Installed>,
    snippets: Option<nocterm_snippets_ui::Installed>,
    pub(crate) themes: nocterm_ui::ThemeUi,
    /// The vault, when it opened; credentials and its settings depend on it.
    pub(crate) vault: Option<Rc<nocterm_vault_ui::Installed>>,
}

impl Booted {
    pub(crate) fn keymap(&self) -> nocterm_keymap::KeymapHandle {
        self.keymap.clone()
    }
    #[cfg(test)]
    pub(crate) fn agent_installation(&self) -> &nocterm_agent::Installed {
        &self.agent
    }
    pub(crate) fn settings_deps(&self) -> nocterm_settings_ui::Installed {
        self.settings_ui.clone()
    }
    pub(crate) fn settings_pages(
        &self,
        guests: Vec<nocterm_ui::SettingsPageSpec>,
    ) -> nocterm_settings_ui::OwnerPages {
        crate::settings_owner_pages(
            self.themes.clone(),
            nocterm_terminal::settings_pages(&self.terminal),
            nocterm_agent::settings_page(&self.agent),
            guests,
        )
    }

    pub(crate) fn workspace_deps(&self) -> nocterm_workspace::Deps {
        self.workspace.clone()
    }
}

/// Composes services and feature dependencies in dependency order.
pub(crate) fn bootstrap(services: Services, cx: &mut App) -> Booted {
    let Services {
        tokens,
        settings,
        paths,
        persist,
        transport,
        local,
        local_host,
        terminal_auth_factory,
        files,
        themes,
        mut agent,
        images,
        vault,
        connections,
    } = services;
    let stored = persist.then_some(&paths);
    gpui_kit::init(cx);
    let base = tokens.clone();
    let ui = nocterm_ui::init(tokens, settings, cx);
    let shutdown = nocterm_store::ShutdownCoordinator::default();
    report_store_registration("settings", ui.settings().register_shutdown(&shutdown));
    let themes = install_themes(themes, ui.settings().clone(), base, cx);
    let layout = nocterm_ui::LayoutMemory::init(
        stored.map(|paths| paths.state_dir().join("layout.json")),
        &shutdown,
        cx,
    );
    let terminal_deps = terminal_dependencies(transport, local, stored, &shutdown);
    let connections = match nocterm_connections::install(connections, &ui, &shutdown, cx) {
        Ok(connections) => Some(connections),
        Err(error) => {
            report_store_registration("connections", Err(error));
            None
        }
    };
    let association_owner = connections
        .as_ref()
        .map(|installed| installed.connections().clone());
    let vault = vault.and_then(|setup| open_vault(setup, &ui, association_owner, cx));
    let (terminal_deps, vault) = match vault {
        Some(opened) => (
            terminal_deps.with_credentials(opened.credentials),
            Some(opened.installed),
        ),
        None => (terminal_deps, None),
    };
    let terminal = nocterm_terminal::install(terminal_deps, &ui, cx);
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
    let settings_ui = nocterm_settings_ui::Installed::new(ui.settings().clone());
    let monitor = nocterm_monitor_ui::install(local_host.clone(), &ui, cx);
    let booted = Booted {
        keymap,
        local_host,
        terminal,
        settings_ui,
        monitor,
        agent,
        connections,
        snippets,
        vault,
        files,
        workspace: nocterm_workspace::Deps::new(layout),
        themes,
    };
    crate::application::register(paths, booted.clone(), ui.settings().clone(), cx);
    booted
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
    match nocterm_snippets_ui::init(paths, settings.clone(), shutdown, cx) {
        Ok(installed) => Some(installed),
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
    stored: Option<&nocterm_core::Paths>,
    shutdown: &nocterm_store::ShutdownCoordinator,
) -> nocterm_terminal::Deps {
    let deps = nocterm_terminal::Deps::new(transport, local);
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
            let association = connections.update(cx, |connections, cx| {
                connections.associate_credential(&spec.target, &spec.auth, id, cx)
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
    let vault = booted.vault.as_deref();
    nocterm_terminal::register(&booted.terminal, workspace);
    if let Some(connections) = &booted.connections {
        nocterm_connections::register(connections, workspace, window, cx);
    }
    crate::register_settings(
        workspace,
        vault,
        booted.themes.clone(),
        booted.settings_deps(),
        booted.keymap(),
        &booted.files,
        &booted.monitor,
        nocterm_terminal::settings_pages(&booted.terminal),
        nocterm_agent::settings_page(&booted.agent),
    );
    nocterm_files::register(&booted.files, workspace, window, cx);
    // The last panel's switcher sits just before the monitor.
    if let Some(snippets) = &booted.snippets {
        nocterm_snippets_ui::register(workspace, snippets, window, cx);
    }
    nocterm_containers_ui::register(booted.local_host.clone(), workspace, window, cx);
    nocterm_monitor_ui::register(&booted.monitor, workspace, window, cx);
    nocterm_agent::register(workspace, booted.agent.clone(), window, cx);
    workspace.set_menu_builder(crate::app_menus::build, window, cx);
    if let Some(vault) = vault
        && vault
            .settings()
            .setting::<nocterm_vault_ui::VaultSettings>(cx)
            .prompt_on_startup
    {
        let pages = vec![nocterm_vault_ui::settings_page(vault)];
        nocterm_settings_ui::open_page(
            workspace,
            "vault",
            &booted.settings_pages(pages),
            booted.settings_deps(),
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;

    #[gpui_kit::test]
    fn configured_observations_use_supplied_disk_cache(cx: &mut gpui_kit::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(directory.path());
        let id = nocterm_connections_model::test_support::profile_id_for("src/bootstrap.rs:1");
        std::fs::create_dir_all(paths.state_dir().join("flags")).unwrap();
        std::fs::write(
            paths.state_dir().join("servers.toml"),
            format!("[server.\"{id}\"]\ncountry = \"xx\"\n"),
        )
        .unwrap();
        std::fs::write(paths.state_dir().join("flags/xx.svg"), b"<svg/>").unwrap();
        let prepared = crate::connection_setup::native(&paths);
        let installed = cx.update(|cx| {
            gpui_kit::init(cx);
            let ready = nocterm_ui::init(
                DesignTokens::builtin(),
                SettingsStore::in_memory(Default::default()),
                cx,
            );
            nocterm_connections::install(
                prepared,
                &ready,
                &nocterm_store::ShutdownCoordinator::default(),
                cx,
            )
            .unwrap()
        });
        cx.run_until_parked();
        assert!(cx.read(|cx| installed.flag("xx", cx).is_some()));
    }

    #[gpui_kit::test]
    fn memory_observations_ignore_even_an_existing_native_cache(cx: &mut gpui_kit::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths::rooted_at(directory.path());
        let id = nocterm_connections_model::test_support::profile_id_for("src/bootstrap.rs:2");
        std::fs::create_dir_all(paths.state_dir().join("flags")).unwrap();
        std::fs::write(
            paths.state_dir().join("servers.toml"),
            format!("[server.\"{id}\"]\ncountry = \"xx\"\n"),
        )
        .unwrap();
        std::fs::write(paths.state_dir().join("flags/xx.svg"), b"<svg/>").unwrap();
        let installed = cx.update(|cx| {
            gpui_kit::init(cx);
            let ready = nocterm_ui::init(
                DesignTokens::builtin(),
                SettingsStore::in_memory(Default::default()),
                cx,
            );
            // The observations are deliberately composed with no configured
            // paths, although old observations are present in the fixture.
            nocterm_connections::install(
                crate::connection_setup::memory(),
                &ready,
                &nocterm_store::ShutdownCoordinator::default(),
                cx,
            )
            .unwrap()
        });
        cx.run_until_parked();
        assert!(cx.read(|cx| installed.flag("xx", cx).is_none()));
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    use futures::executor::block_on;
    use gpui_kit::AppContext as _;
    use nocterm_session::{
        Auth, CredentialStore as _, Event, Prompt, Reply, Secret, SecretRequest, Target,
        testing::ScriptedTransport,
    };

    fn history_profile(target: Target) -> nocterm_connections_model::profiles::Profile {
        nocterm_connections_model::profiles::Profile {
            id: nocterm_connections_model::profiles::ProfileId::from_bytes([7; 16]),
            name: "History".into(),
            description: String::new(),
            options: Default::default(),
            target,
            auth: Auth::Password,
            credential: None,
            launch: None,
            icon: None,
            icon_color: None,
            country: None,
            group: None,
        }
    }
    fn association_fixture(
        profile: nocterm_connections_model::profiles::Profile,
        file: std::path::PathBuf,
        cx: &mut gpui_kit::TestAppContext,
    ) -> (
        gpui_kit::Entity<nocterm_terminal::Terminal>,
        Arc<dyn VaultAccess>,
        gpui_kit::Entity<nocterm_connections::Connections>,
        nocterm_session::SessionDriver,
    ) {
        let mut profiles = nocterm_connections_model::profiles::Profiles::default();
        profiles.upsert(profile.clone());
        let transport = Arc::new(ScriptedTransport::default());
        let driver = transport.queue(None, None);
        let (terminal, service, connections) = cx.update(|cx| {
            gpui_kit::init(cx);
            let ready = nocterm_ui::init(
                DesignTokens::builtin(),
                SettingsStore::in_memory(Default::default()),
                cx,
            );
            let connections = cx.new(|_| {
                nocterm_connections::Connections::with_backends(
                    profiles,
                    Default::default(),
                    Arc::new(|_| {
                        Box::pin(async {
                            Err(nocterm_store::StoreError::backend(std::io::Error::new(
                                std::io::ErrorKind::PermissionDenied,
                                "history refused",
                            )))
                        })
                    }),
                    Arc::new(|_| Box::pin(async { Ok(()) })),
                )
            });
            let opened = open_vault(
                VaultSetup {
                    file,
                    device_unlock: false,
                },
                &ready,
                Some(connections.clone()),
                cx,
            )
            .unwrap();
            let service = opened.installed.service().clone();
            let installed = nocterm_terminal::install(
                nocterm_terminal::Deps::new(
                    transport,
                    nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
                )
                .with_credentials(opened.credentials),
                &ready,
                cx,
            );
            let terminal = cx.new(|cx| {
                nocterm_terminal::Terminal::new(
                    installed,
                    nocterm_workspace::SessionSpec {
                        profile: Some(profile.id.to_string().into()),
                        options: Default::default(),
                        title: "History".into(),
                        target: profile.target.clone(),
                        auth: Auth::Password,
                        launch: None,
                        credential: None,
                    },
                    cx,
                )
            });
            (terminal, service, connections)
        });
        (terminal, service, connections, driver)
    }
    #[gpui_kit::test]
    async fn actual_connection_callback_refusal_keeps_the_authenticated_vault_credential(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let directory = tempfile::tempdir().unwrap();
        let target = Target::new("user", "host", 22);
        let profile = history_profile(target.clone());
        let (terminal, service, connections, driver) =
            association_fixture(profile.clone(), directory.path().join("vault"), cx);
        block_on(service.create(Secret::new("long master password"))).unwrap();
        let request = SecretRequest::Password {
            target,
            retry: false,
        };
        let (reply, answer) = Reply::channel();
        assert!(block_on(driver.emit(Event::Prompt(Prompt::Secret {
            request: request.clone(),
            reply
        }))));
        cx.run_until_parked();
        cx.update(|cx| {
            terminal.update(cx, |terminal, cx| {
                terminal.answer_secret_and_remember(Secret::new("account password"), true, cx)
            })
        });
        assert_eq!(
            block_on(answer).unwrap().unwrap().expose(),
            "account password"
        );
        assert!(block_on(service.list()).unwrap().is_empty());
        assert!(block_on(driver.emit(Event::Connected)));
        terminal
            .condition::<nocterm_terminal::TerminalEvent>(cx, |terminal, _| {
                terminal
                    .credential_message()
                    .is_some_and(|message| message.contains("history refused"))
            })
            .await;
        let id = cx.update(|cx| terminal.read(cx).spec().credential.unwrap());
        let credentials = nocterm_vault_ui::VaultCredentials::new(service);
        assert_eq!(
            block_on(credentials.get(id, &request)).unwrap().expose(),
            "account password"
        );
        assert!(cx.update(|cx| {
            terminal
                .read(cx)
                .credential_message()
                .unwrap()
                .contains("history refused")
        }));
        assert!(connections.read_with(cx, |connections, cx| {
            connections
                .profiles(cx)
                .get(profile.id)
                .unwrap()
                .credential
                .is_none()
        }));
    }
}
