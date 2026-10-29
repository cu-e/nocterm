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
use nocterm_store::SettingsConfig;

use crate::{AGENT_BRIDGE, agent_auth, bootstrap, shell_setup::UserPluginMode};

pub(crate) struct PreparedApp {
    paths: Paths,
    executable: PathBuf,
    tokens: DesignTokens,
    settings: SettingsConfig,
    themes: bootstrap::Themes,
    hosts: bootstrap::HostServices,
    files: nocterm_files::Deps,
    local_fs: Arc<dyn nocterm_session::local_fs::LocalFs>,
    user_plugins: UserPluginMode,
}
impl PreparedApp {
    pub(crate) fn load(user_plugins: UserPluginMode) -> anyhow::Result<Self> {
        // Linux reports a replaced running binary as "<path> (deleted)".
        let executable = std::env::current_exe().context("locating the nocterm executable")?;
        let paths = Paths::discover()?;
        let tokens = load_tokens(&paths);
        let settings = load_settings(&paths, &user_plugins);
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
        let hub_links = hub_links(&paths);
        let transport = ssh_transport(&paths, hub_links.as_deref())?;
        let local_fs = Arc::new(
            nocterm_localfs::LocalFilesystem::new().context("opening the local filesystem")?,
        );
        let local = local_sessions();
        let local_host = local
            .is_available()
            .then(|| Arc::new(nocterm_local::LocalExec) as Arc<dyn nocterm_session::HostExec>);
        let activation = Arc::new(nocterm_project_environment::NativeWorkspaceSessions);
        let environment = Arc::new(
            nocterm_project_environment::NativeWorkspaceEnvironment::new(Arc::new(
                nocterm_project_runtime::NativeEnvironmentCommands,
            )),
        );
        let preparation = Arc::new(nocterm_project_runtime::NativeProjectPreparation::new(
            environment.clone(),
        ));
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
            hosts: bootstrap::HostServices {
                transport: Arc::new(transport),
                local,
                local_host,
                image_search: Arc::new(nocterm_project_runtime::NativeImageSearch),
                environment,
                preparation,
                container_sessions: Arc::new(
                    nocterm_project_container_fs::NativeContainerSessions::new(activation.clone()),
                ),
                workspace_sessions: activation,
                container_actions: Arc::new(
                    nocterm_project_runtime::NativeWorkspaceContainerActions,
                ),
                hubs: hub_dependencies(hub_links.clone()),
                hub_accounts: hub_links
                    .map(|links| links as Arc<dyn crate::hub_sources::HubAccounts>),
            },
            files: files_dependencies(local_fs.clone())?,
            local_fs,
            user_plugins,
        })
    }

    pub(crate) fn install_when_ready(self, cx: &mut App) {
        let state = Arc::new(nocterm_agent_storage::FileAgentStateRepository::new(
            self.paths.state_dir().join("agents.toml"),
        ));
        let chats = Arc::new(nocterm_agent_storage::FileChatRepository::new(
            self.paths.state_dir().join("agent-chats"),
            self.local_fs.clone(),
        ));
        let paths = self.paths.clone();
        let loading = cx.background_executor().spawn(async move {
            let connections = crate::connection_setup::native(&paths);
            let projects = crate::project_setup::PreparedProjectSetup::native(&paths);
            let storage = nocterm_agent_runtime::AgentStorage::prepare(state, chats).await;
            (storage, connections, projects)
        });
        cx.spawn(async move |cx| {
            let (storage, connections, projects) = loading.await;
            cx.update(|cx| self.finish(storage, connections, projects, cx));
        })
        .detach();
    }

    fn finish(
        self,
        storage: nocterm_agent_runtime::AgentStorage,
        connections: nocterm_connections::PreparedConnections,
        projects: crate::project_setup::PreparedProjectSetup,
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
                paths: self.paths,
                persist: true,
                hosts: self.hosts,
                terminal_auth_factory: Some(agent_auth::opener),
                files: self.files,
                themes: Some(self.themes),
                agent,
                sandbox_probe: Arc::new(nocterm_acp::NativeSandboxProbe),
                vault,
                connections,
                projects,
                user_plugins: self.user_plugins,
            },
            cx,
        );
        let booted = match booted {
            Ok(booted) => booted,
            Err(error) => {
                tracing::error!(?error, "could not compose the application");
                cx.quit();
                return;
            }
        };
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
        crate::report_settings_errors(booted.settings_deps().settings(), cx);
        cx.activate(true);
    }
}

fn files_dependencies(
    local_fs: Arc<nocterm_localfs::LocalFilesystem>,
) -> anyhow::Result<nocterm_files::Deps> {
    let transfers = nocterm_transfer_runtime::TransferRuntime::start(
        nocterm_transfer_runtime::TransferRuntimeDeps::new(
            local_fs.clone(),
            nocterm_transfers::TransferLimits::default(),
        ),
    )
    .context("starting transfers")?;
    let directories = Arc::new(nocterm_directory_io::DirectoryReaders::new(
        local_fs.clone(),
    ));
    Ok(nocterm_files::Deps::new(
        local_fs.clone(),
        local_fs,
        transfers,
        Arc::new(nocterm_local::LocalProgramOpener::default()),
        directories,
    ))
}

/// The adapter that reaches hubs, or none when it cannot start: local work does not suffer.
fn hub_links(paths: &Paths) -> Option<Arc<nocterm_hub_io::HubLinks>> {
    let config = nocterm_hub_io::HubLinksConfig {
        credentials_dir: paths.state_dir().join("hubs"),
        device_name: device_name(),
    };
    match nocterm_hub_io::HubLinks::start(config) {
        Ok(links) => Some(links),
        Err(error) => {
            tracing::error!(?error, "hubs are unavailable");
            None
        }
    }
}

/// The hub access ports of the links, if there are any.
fn hub_dependencies(links: Option<Arc<nocterm_hub_io::HubLinks>>) -> Option<nocterm_hubs::Deps> {
    links.map(|links| nocterm_hubs::Deps {
        trust: links.clone(),
        sign_in: links.clone(),
        admin_keys: links,
    })
}

/// The SSH transport. A profile that routes its first hop through a hub is dialled by the hub
/// links; without links such a profile fails to connect and everything else is unaffected.
fn ssh_transport(
    paths: &Paths,
    links: Option<&nocterm_hub_io::HubLinks>,
) -> anyhow::Result<SshTransport> {
    let transport = SshTransport::new(ssh_config(paths)).context("starting the SSH runtime")?;
    Ok(match links {
        Some(links) => transport.with_relay(Arc::new(nocterm_hub_io::HubRelay::new(links))),
        None => transport,
    })
}

/// How this computer is named in the device list of a hub.
fn device_name() -> String {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .or_else(|| fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Nocterm".to_owned())
}

fn local_sessions() -> nocterm_terminal::LocalSessions {
    let program = nocterm_local::default_shell();
    let supplied = program.clone();
    nocterm_terminal::LocalSessions::new(
        Arc::new(move |mut launch| {
            if launch.program.is_none() {
                launch.program = Some(supplied.clone());
            }
            Arc::new(nocterm_local::LocalTransport(launch))
        }),
        program,
    )
}

fn agent_services(
    paths: &Paths,
    executable: std::path::PathBuf,
    storage: nocterm_agent_runtime::AgentStorage,
) -> nocterm_agent::AgentServices {
    nocterm_agent::AgentServices {
        command_jobs: Arc::new(nocterm_agent_tools::JobsFactory),
        catalog: nocterm_agent_providers::catalog(),
        metadata: Arc::new(nocterm_agent_storage::NativeChatMetadata),
        connector: Arc::new(nocterm_acp::AcpConnector::managed(executable.clone())),
        terminal_auth: None,
        bridge: Arc::new(nocterm_acp::BridgeServer::new(
            paths.clone(),
            nocterm_acp::RelayCommand {
                program: executable,
                args: vec![AGENT_BRIDGE.into()],
            },
        )),
        storage,
        workdir_resolver: Arc::new(nocterm_acp::NativeWorkdirResolver::default()),
        workdir: paths.state_dir().join("agent-workspace"),
        private_dirs: vec![paths.config_dir().to_owned(), paths.state_dir().to_owned()],
        shared_dirs: vec![paths.effective_runtime_dir()],
    }
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

/// Invalid sections keep their source; malformed files disable writes for this run. A
/// developer session reads the file and never writes it.
fn load_settings(paths: &Paths, mode: &UserPluginMode) -> SettingsConfig {
    let file = SettingsFile::new(paths.settings_file());
    match file.load() {
        Ok(document) if mode.saves_settings() => SettingsConfig::new(document, file),
        Ok(document) => SettingsConfig::in_memory(document),
        Err(error) => {
            tracing::error!(%error, "could not read the settings; changes will not be saved");
            SettingsConfig::in_memory(SettingsDocument::default())
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

#[cfg(test)]
mod tests;
