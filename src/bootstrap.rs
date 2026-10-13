//! The one order the application's globals are installed in, shared by the
//! application and its GUI tests so the tests exercise the real wiring.

use std::sync::Arc;

use gpui_kit::{App, Context, Window};
use nocterm_core::Paths;
use nocterm_design::DesignTokens;
use nocterm_session::Transport;
use nocterm_ui::{SettingsExt as _, SettingsStore};
use nocterm_vault::VaultAccess;
use nocterm_vault_store::VaultService;
use nocterm_workspace::Workspace;
use std::rc::Rc;

/// What differs between the application and its tests.
pub(crate) struct Services {
    pub(crate) tokens: DesignTokens,
    pub(crate) settings: SettingsStore,
    pub(crate) settings_ui: nocterm_settings_ui::Deps,
    pub(crate) paths: Paths,
    /// Whether connections, snippets, layout, recordings and the keymap are
    /// read from and written to `paths`; tests keep them in memory.
    pub(crate) persist: bool,
    pub(crate) transport: Arc<dyn Transport>,
    /// Whether local shells run on this machine.
    pub(crate) local: bool,
    pub(crate) files: nocterm_files::Deps,
    pub(crate) themes: Option<Themes>,
    pub(crate) agent: nocterm_agent::AgentServices,
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

/// What the installed globals leave for the windows.
#[derive(Clone)]
pub(crate) struct Booted {
    settings_ui: nocterm_settings_ui::Deps,
    files: nocterm_files::Installed,
    workspace: nocterm_workspace::Deps,
    pub(crate) themes: nocterm_ui::ThemeUi,
    /// The vault, when it opened; credentials and its settings depend on it.
    pub(crate) vault: Option<Rc<nocterm_vault_ui::Installed>>,
}

impl Booted {
    pub(crate) fn settings_deps(&self) -> nocterm_settings_ui::Deps {
        self.settings_ui.clone()
    }
    pub(crate) fn workspace_deps(&self) -> nocterm_workspace::Deps {
        self.workspace.clone()
    }
}

/// Installs every global, in dependency order.
pub(crate) fn bootstrap(services: Services, cx: &mut App) -> Booted {
    let Services {
        tokens,
        settings,
        settings_ui,
        paths,
        persist,
        transport,
        local,
        files,
        themes,
        agent,
        vault,
        connections,
    } = services;
    let stored = persist.then_some(&paths);
    gpui_kit::init(cx);
    let base = tokens.clone();
    let ui = nocterm_ui::init(tokens, settings, cx);
    let shutdown = nocterm_store::ShutdownCoordinator::default();
    report_store_registration("settings", ui.settings().register_shutdown(&shutdown));
    let (library, registry, catalog) = match themes {
        Some(themes) => (Some(themes.library), Some(themes.registry), themes.catalog),
        None => (None, None, nocterm_themes::ThemeCatalog::default()),
    };
    let themes = nocterm_theme_service::ThemesHandle::install(library, registry, catalog, cx);
    let themes = nocterm_ui::ThemeUi::install(ui.settings().clone(), themes, base, cx);
    let layout = nocterm_ui::LayoutMemory::init(
        stored.map(|paths| paths.state_dir().join("layout.json")),
        &shutdown,
        cx,
    );
    nocterm_terminal::init(transport, &ui, cx);
    if persist {
        let log_factory = Arc::new(nocterm_local::LocalOutputLogFactory::new(
            paths.state_dir().join("logs"),
        ));
        let registry = Arc::new(nocterm_store::RecordingRegistry::new(log_factory));
        report_store_registration("recording", registry.register(&shutdown));
        nocterm_terminal::init_recording(registry, cx);
    }
    if local {
        nocterm_terminal::init_local(
            Arc::new(|launch| Arc::new(nocterm_local::LocalTransport(launch))),
            cx,
        );
        nocterm_workspace::host::set_local_exec(Arc::new(nocterm_local::LocalExec), cx);
    }
    report_store_registration(
        "connections",
        nocterm_connections::init(connections, &shutdown, cx).map(|_| ()),
    );
    report_store_registration("snippets", nocterm_snippets_ui::init(stored, &shutdown, cx));
    report_store_registration("agent", nocterm_agent::init(agent, &ui, &shutdown, cx));
    let files = nocterm_files::install(files, &ui, cx);
    register_transfer_shutdown(&files, &shutdown);
    let vault = vault.and_then(|setup| open_vault(setup, &ui, cx));
    crate::keymap::load(
        stored.map(|paths| paths.config_dir().join("keymap.toml")),
        cx,
    );
    let booted = Booted {
        settings_ui,
        vault,
        files,
        workspace: nocterm_workspace::Deps::new(layout),
        themes,
    };
    crate::application::register(paths, booted.clone(), cx);
    booted
}

/// Freeze transfer admission during synchronous capture, then drain without App.
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

/// Opens the vault and lets terminals keep credentials in it.
fn open_vault(
    setup: VaultSetup,
    ready: &nocterm_ui::UiReady,
    cx: &mut App,
) -> Option<Rc<nocterm_vault_ui::Installed>> {
    let device = setup.device_unlock.then(nocterm_device_unlock::provider);
    nocterm_ui::register_setting::<nocterm_vault_ui::VaultSettings>(cx);
    let duration = std::time::Duration::from_secs(
        u64::from(
            cx.setting::<nocterm_vault_ui::VaultSettings>()
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
    nocterm_terminal::init_credentials(
        Arc::new(nocterm_vault_ui::VaultCredentials::new(vault.clone())),
        |spec, id, cx| {
            let association =
                nocterm_connections::Connections::global(cx).update(cx, |connections, cx| {
                    connections.associate_credential(&spec.target, &spec.auth, id, cx)
                });
            cx.spawn(async move |_| {
                association
                    .await
                    .map_err(nocterm_terminal::CredentialAssociationError::binding)
            })
        },
        cx,
    );
    Some(Rc::new(nocterm_vault_ui::install(
        nocterm_vault_ui::Deps::new(vault),
        ready,
        cx,
    )))
}

/// Plugs every feature into a new window's workspace.
pub(crate) fn build_workspace(
    workspace: &mut Workspace,
    booted: &Booted,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let vault = booted.vault.as_deref();
    workspace.set_session_factory(nocterm_terminal::Factory);
    nocterm_connections::register(workspace, window, cx);
    crate::register_settings(
        workspace,
        vault,
        booted.themes.clone(),
        booted.settings_deps(),
    );
    nocterm_files::register(&booted.files, workspace, window, cx);
    // The last panel's switcher sits just before the monitor.
    nocterm_snippets_ui::register(workspace, window, cx);
    nocterm_containers_ui::register(workspace, window, cx);
    nocterm_monitor_ui::register(workspace, window, cx);
    nocterm_agent::register(workspace, window, cx);
    workspace.set_menu_builder(crate::app_menus::build, window, cx);
    if let Some(vault) = vault
        && cx
            .setting::<nocterm_vault_ui::VaultSettings>()
            .prompt_on_startup
    {
        let pages = vec![nocterm_vault_ui::settings_page(vault)];
        nocterm_settings_ui::open_page(
            workspace,
            "vault",
            &pages,
            booted.themes.clone(),
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
        cx.update(|cx| {
            gpui_kit::init(cx);
            nocterm_connections::init(prepared, &nocterm_store::ShutdownCoordinator::default(), cx)
                .unwrap();
        });
        cx.run_until_parked();
        let facts = cx.read(nocterm_connections::ServerFacts::global);
        assert!(facts.read_with(cx, |facts, _| facts.flag("xx").is_some()));
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
        cx.update(|cx| {
            gpui_kit::init(cx);
            // The observations are deliberately composed with no configured
            // paths, although old observations are present in the fixture.
            nocterm_connections::init(
                crate::connection_setup::memory(),
                &nocterm_store::ShutdownCoordinator::default(),
                cx,
            )
            .unwrap();
        });
        cx.run_until_parked();
        let facts = cx.read(nocterm_connections::ServerFacts::global);
        assert!(facts.read_with(cx, |facts, _| facts.flag("xx").is_none()));
    }
}
