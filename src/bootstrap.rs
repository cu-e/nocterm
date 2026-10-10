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
    pub(crate) paths: Paths,
    /// Whether connections, snippets, layout, recordings and the keymap are
    /// read from and written to `paths`; tests keep them in memory.
    pub(crate) persist: bool,
    pub(crate) transport: Arc<dyn Transport>,
    /// Whether local shells run on this machine.
    pub(crate) local: bool,
    pub(crate) themes: Option<Themes>,
    pub(crate) agent: nocterm_agent::AgentServices,
    pub(crate) vault: Option<VaultSetup>,
}

pub(crate) struct Themes {
    pub(crate) dirs: nocterm_themes::ThemeDirs,
    pub(crate) catalog: nocterm_themes::ThemeCatalog,
    pub(crate) registry: Arc<nocterm_themes::ZedRegistry>,
}

pub(crate) struct VaultSetup {
    pub(crate) file: std::path::PathBuf,
    /// Whether the platform's authenticated unlock is offered.
    pub(crate) device_unlock: bool,
}

/// What the installed globals leave for the windows.
pub(crate) struct Booted {
    /// The vault, when it opened; credentials and its settings depend on it.
    pub(crate) vault: Option<Rc<nocterm_vault_ui::Installed>>,
}

/// Installs every global, in dependency order.
pub(crate) fn bootstrap(services: Services, cx: &mut App) -> Booted {
    let Services {
        tokens,
        settings,
        paths,
        persist,
        transport,
        local,
        themes,
        agent,
        vault,
    } = services;
    let stored = persist.then_some(&paths);
    gpui_kit::init(cx);
    let ui = nocterm_ui::init(tokens, settings, cx);
    let shutdown = nocterm_store::ShutdownCoordinator::default();
    report_store_registration("settings", ui.settings().register_shutdown(&shutdown));
    if let Some(themes) = themes {
        nocterm_ui::init_themes(themes.dirs, themes.catalog, cx);
        nocterm_settings_ui::init_theme_registry(themes.registry, cx);
    }
    nocterm_ui::LayoutMemory::init(
        stored.map(|paths| paths.state_dir().join("layout.json")),
        &shutdown,
        cx,
    );
    nocterm_terminal::init(transport, &ui, cx);
    if persist {
        nocterm_terminal::init_recording(paths.state_dir().join("logs"), cx);
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
        nocterm_connections::init(stored, &shutdown, cx),
    );
    report_store_registration("snippets", nocterm_snippets_ui::init(stored, &shutdown, cx));
    nocterm_agent::init(agent, &ui, cx);
    let vault = vault.and_then(|setup| open_vault(setup, &ui, cx));
    crate::keymap::load(
        stored.map(|paths| paths.config_dir().join("keymap.toml")),
        cx,
    );
    crate::application::register(paths, vault.clone(), cx);
    Booted { vault }
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
    vault: Option<&nocterm_vault_ui::Installed>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    workspace.set_session_factory(nocterm_terminal::Factory);
    nocterm_connections::register(workspace, window, cx);
    crate::register_settings(workspace, vault);
    nocterm_files::register(workspace, window, cx);
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
        nocterm_settings_ui::open_page(workspace, "vault", &pages, window, cx);
    }
}
