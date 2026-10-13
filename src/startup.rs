//! Application inputs prepared once, then installed after preference hydration.
//!
//! Owns native collaborator construction and the transition to the first window.
//! Files and their transfer/opener handles are created once and retained across
//! async agent-state loading; bootstrap supplies the one shutdown coordinator.

use std::{fs, io, path::PathBuf, sync::Arc};

use anyhow::Context as _;
use gpui_kit::App;
use nocterm_core::Paths;
use nocterm_design::DesignTokens;
use nocterm_settings::{SettingsDocument, SettingsFile};
use nocterm_ssh::{SshConfig, SshTransport};
use nocterm_ui::SettingsStore;

use crate::{AGENT_BRIDGE, agent_auth, bootstrap};

pub(crate) struct PreparedApp {
    paths: Paths,
    executable: PathBuf,
    tokens: DesignTokens,
    settings: SettingsStore,
    themes: bootstrap::Themes,
    transport: SshTransport,
    files: nocterm_files::Deps,
}
impl PreparedApp {
    pub(crate) fn load() -> anyhow::Result<Self> {
        // Linux reports a replaced running binary as "<path> (deleted)".
        let executable = std::env::current_exe().context("locating the nocterm executable")?;
        let paths = Paths::discover()?;
        let tokens = load_tokens(&paths);
        let settings = load_settings(&paths);
        let dirs = nocterm_themes::ThemeDirs {
            user: paths.user_themes_dir(),
            installed: paths.installed_themes_dir(),
        };
        let registry = Arc::new(
            nocterm_themes_registry::ZedRegistry::new()
                .context("creating the theme registry client")?,
        );
        let library: Arc<dyn nocterm_themes::ThemeLibrary> = Arc::new(
            nocterm_themes_registry::LocalThemeLibrary::new(dirs, Some(registry.clone())),
        );
        let catalog = library.scan();
        let transport =
            SshTransport::new(ssh_config(&paths)).context("starting the SSH runtime")?;
        Ok(Self {
            paths,
            executable,
            tokens,
            settings,
            themes: bootstrap::Themes {
                library,
                catalog,
                registry,
            },
            transport,
            files: files_dependencies()?,
        })
    }

    pub(crate) fn install_when_ready(self, cx: &mut App) {
        let state = Arc::new(nocterm_agent_storage::FileAgentStateRepository::new(
            self.paths.state_dir().join("agents.toml"),
        ));
        let chats = Arc::new(nocterm_agent_storage::FileChatRepository::new(
            self.paths.state_dir().join("agent-chats"),
        ));
        let paths = self.paths.clone();
        let loading = cx.background_executor().spawn(async move {
            let connections = crate::connection_setup::native(&paths);
            let storage = nocterm_agent_runtime::AgentStorage::prepare(state, chats).await;
            (storage, connections)
        });
        cx.spawn(async move |cx| {
            let (storage, connections) = loading.await;
            cx.update(|cx| self.finish(storage, connections, cx));
        })
        .detach();
    }

    fn finish(
        self,
        storage: nocterm_agent_runtime::AgentStorage,
        connections: nocterm_connections::PreparedConnections,
        cx: &mut App,
    ) {
        let agent = agent_services(&self.paths, self.executable, storage);
        let vault = Some(bootstrap::VaultSetup {
            file: self.paths.config_dir().join("vault.bin"),
            device_unlock: true,
        });
        let codec = Arc::new(nocterm_image_codec::NativeImageCodec);
        let images = nocterm_agent::ImageDeps::new(codec.clone(), codec);
        let booted = bootstrap::bootstrap(
            bootstrap::Services {
                images,
                tokens: self.tokens,
                settings: self.settings,
                settings_ui: nocterm_settings_ui::Deps::new(Arc::new(
                    nocterm_acp::NativeSandboxProbe,
                )),
                paths: self.paths,
                persist: true,
                transport: Arc::new(self.transport),
                local: true,
                files: self.files,
                themes: Some(self.themes),
                agent,
                vault,
                connections,
            },
            cx,
        );
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if let Err(error) = crate::open_main_window(cx, &booted) {
            tracing::error!(%error, "could not open the window");
            cx.quit();
        }
        crate::report_settings_errors(cx);
        cx.activate(true);
    }
}

fn files_dependencies() -> anyhow::Result<nocterm_files::Deps> {
    let local_fs =
        Arc::new(nocterm_localfs::LocalFilesystem::new().context("opening the local filesystem")?);
    let transfers = nocterm_transfer_runtime::TransferRuntime::start(
        nocterm_transfer_runtime::TransferRuntimeDeps::new(
            local_fs.clone(),
            nocterm_transfers::TransferLimits::default(),
        ),
    )
    .context("starting transfers")?;
    Ok(nocterm_files::Deps::new(
        local_fs.clone(),
        local_fs,
        transfers,
        Arc::new(nocterm_local::LocalProgramOpener::default()),
    ))
}

fn agent_services(
    paths: &Paths,
    executable: std::path::PathBuf,
    storage: nocterm_agent_runtime::AgentStorage,
) -> nocterm_agent::AgentServices {
    nocterm_agent::AgentServices {
        metadata: Arc::new(nocterm_agent_storage::NativeChatMetadata),
        connector: Arc::new(nocterm_acp::AcpConnector::managed(executable.clone())),
        terminal_auth: Some(Arc::new(agent_auth::open)),
        bridge: Arc::new(nocterm_acp::BridgeServer::new(
            paths.clone(),
            nocterm_acp::RelayCommand {
                program: executable,
                args: vec![AGENT_BRIDGE.into()],
            },
        )),
        storage,
        workdir_resolver: Arc::new(nocterm_acp::NativeWorkdirResolver::default()),
        usage_source: Arc::new(nocterm_agent_storage::CodexUsageReader::new(codex_home())),
        workdir: paths.state_dir().join("agent-workspace"),
        private_dirs: vec![paths.config_dir().to_owned(), paths.state_dir().to_owned()],
        shared_dirs: vec![paths.effective_runtime_dir()],
    }
}

/// Resolve the native account-log location once at composition.
fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
}

/// A broken theme is reported and ignored so the window still opens.
fn load_tokens(paths: &Paths) -> DesignTokens {
    let file = paths.theme_file();
    let overrides = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return DesignTokens::builtin(),
        Err(error) => {
            tracing::error!(file = %file.display(), %error, "could not read the theme");
            return DesignTokens::builtin();
        }
    };
    DesignTokens::with_overrides(&overrides).unwrap_or_else(|error| {
        tracing::error!(file = %file.display(), %error, "the theme is invalid; using the default");
        DesignTokens::builtin()
    })
}

/// Invalid sections keep their source; malformed files disable writes for this run.
fn load_settings(paths: &Paths) -> SettingsStore {
    let file = SettingsFile::new(paths.settings_file());
    match file.load() {
        Ok(document) => SettingsStore::new(document, file),
        Err(error) => {
            tracing::error!(%error, "could not read the settings; changes will not be saved");
            SettingsStore::in_memory(SettingsDocument::default())
        }
    }
}

fn ssh_config(paths: &Paths) -> SshConfig {
    let openssh = paths.openssh_dir();
    SshConfig {
        known_hosts: paths.known_hosts_file(),
        read_only_known_hosts: openssh.iter().map(|dir| dir.join("known_hosts")).collect(),
        identity_dir: openssh,
        use_agent: true,
    }
}
