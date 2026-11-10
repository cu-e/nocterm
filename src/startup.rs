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
    /// The Files transfer runtime, shared so agent transfers join the user's activity.
    agent_transfers: Arc<dyn nocterm_transfers::TransferRequests>,
    local_fs: Arc<dyn nocterm_session::local_fs::LocalFs>,
    /// Hub chats are listed beside the local history and run by the hub connector.
    hub_links: Option<Arc<nocterm_hub_io::HubLinks>>,
    user_plugins: UserPluginMode,
}
impl PreparedApp {
    pub(crate) fn load(user_plugins: UserPluginMode) -> anyhow::Result<Self> {
        // Linux reports a replaced running binary as "<path> (deleted)".
        let executable = std::env::current_exe().context("locating the nocterm executable")?;
        let paths = Paths::discover()?;
        let tokens = load_tokens(&paths);
        let mut settings = load_settings(&paths, &user_plugins);
        settings.register::<nocterm_hubs_model::HubsSettings>();
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
        let hub_links = hub_links(&paths, settings.document().get());
        let agent_hosts = Arc::new(nocterm_ssh::authorized_keys::HostDirectory::new(
            &ssh_config(&paths),
        ));
        let transport = ssh_transport(&paths, hub_links.as_deref())?;
        let local_fs = Arc::new(
            nocterm_localfs::LocalFilesystem::new().context("opening the local filesystem")?,
        );
        let local = local_sessions();
        let local_host = local.is_available().then(|| {
            Arc::new(nocterm_local::LocalExec::default()) as Arc<dyn nocterm_session::HostExec>
        });
        let activation = Arc::new(nocterm_project_environment::NativeWorkspaceSessions);
        let environment = Arc::new(
            nocterm_project_environment::NativeWorkspaceEnvironment::new(Arc::new(
                nocterm_project_runtime::NativeEnvironmentCommands,
            )),
        );
        let preparation = Arc::new(nocterm_project_runtime::NativeProjectPreparation::new(
            environment.clone(),
        ));
        let (files, agent_transfers) = files_dependencies(local_fs.clone())?;
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
                hubs: hub_links.clone().map(hub_dependencies),
                hub_agents: hub_links
                    .as_deref()
                    .map(|links| hub_agents_dependencies(links, agent_hosts.clone())),
                agent_hosts: hub_links.is_some().then_some(agent_hosts as _),
                hub_accounts: hub_links
                    .clone()
                    .map(|links| links as Arc<dyn crate::hub_sources::HubAccounts>),
            },
            files,
            agent_transfers,
            local_fs,
            hub_links,
            user_plugins,
        })
    }

    pub(crate) fn install_when_ready(self, cx: &mut App) {
        let state = Arc::new(nocterm_agent_storage::FileAgentStateRepository::new(
            self.paths.state_dir().join("agents.toml"),
        ));
        let local_chats: Arc<dyn nocterm_ai::storage::ChatRepository> =
            Arc::new(nocterm_agent_storage::FileChatRepository::new(
                self.paths.state_dir().join("agent-chats"),
                self.local_fs.clone(),
            ));
        let chats = match &self.hub_links {
            Some(links) => Arc::new(nocterm_hub_io::HubChatRepository::new(
                links,
                local_chats,
                self.paths.state_dir().join("hub-chats"),
            )),
            None => local_chats,
        };
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
        let connector = agent_connector(&self.executable, self.hub_links.as_deref());
        let agent = agent_services(
            &self.paths,
            self.executable,
            connector,
            self.agent_transfers,
            storage,
        );
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
        crate::report_settings_errors(&booted.settings_deps(), cx);
        cx.activate(true);
    }
}

/// Files dependencies and their transfer runtime, which agents submit to as well.
fn files_dependencies(
    local_fs: Arc<nocterm_localfs::LocalFilesystem>,
) -> anyhow::Result<(
    nocterm_files::Deps,
    Arc<dyn nocterm_transfers::TransferRequests>,
)> {
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
    let files = nocterm_files::Deps::new(
        local_fs.clone(),
        local_fs,
        transfers.clone(),
        Arc::new(nocterm_local::LocalProgramOpener::default()),
        directories,
    );
    Ok((files, transfers))
}

/// The adapter that reaches hubs, or none when it cannot start: local work does not suffer.
fn hub_links(
    paths: &Paths,
    hubs: &nocterm_hubs_model::HubsSettings,
) -> Option<Arc<nocterm_hub_io::HubLinks>> {
    let config = nocterm_hub_io::HubLinksConfig {
        credentials_dir: paths.state_dir().join("hubs"),
        device_name: device_name(),
    };
    let links = match nocterm_hub_io::HubLinks::start(config) {
        Ok(links) => links,
        Err(error) => {
            tracing::error!(?error, "hubs are unavailable");
            return None;
        }
    };
    // PreparedApp::load runs before the GPUI application starts. Known hubs'
    // first cache reads belong here; composing their windows only reuses Slots.
    for hub in &hubs.servers {
        if let Err(error) = links.open_account(&hub.id()) {
            tracing::warn!(%error, "the known hub account cache could not be prepared");
        }
    }
    Some(links)
}

/// The hub access ports of the links.
pub(crate) fn hub_dependencies(links: Arc<nocterm_hub_io::HubLinks>) -> nocterm_hubs::Deps {
    nocterm_hubs::Deps {
        trust: links.clone(),
        sign_in: links.clone(),
        devices: links.clone(),
        account_keys: Arc::new(nocterm_hub_io::AccountKeys::new(links)),
    }
}

/// The agents, agent servers and chat changes of the hubs, all read by one link, and the
/// sessions of saved connections Agent access writes `authorized_keys` through.
pub(crate) fn hub_agents_dependencies(
    links: &nocterm_hub_io::HubLinks,
    hosts: Arc<nocterm_ssh::authorized_keys::HostDirectory>,
) -> nocterm_hub_agents::Deps {
    let link = Arc::new(nocterm_hub_io::HubAgentsLink::new(links));
    nocterm_hub_agents::Deps {
        directory: link.clone(),
        servers: link.clone(),
        changes: link,
        hosts,
    }
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
        Arc::new(move |mut launch, preparation| {
            if launch.program.is_none() {
                launch.program = Some(supplied.clone());
            }
            nocterm_local::LocalTransport::prepared(launch, preparation)
        }),
        program,
    )
}

/// Local agents run over ACP; hub agents run on their hub.
pub(crate) fn agent_connector(
    executable: &std::path::Path,
    links: Option<&nocterm_hub_io::HubLinks>,
) -> Arc<dyn nocterm_ai::AgentConnector> {
    Arc::new(crate::agent_connector::AgentConnectors::new(
        Arc::new(nocterm_acp::AcpConnector::managed(executable.to_owned())),
        links.map(|links| {
            Arc::new(nocterm_hub_io::HubAgentConnector::new(links))
                as Arc<dyn nocterm_ai::AgentConnector>
        }),
    ))
}

fn agent_services(
    paths: &Paths,
    executable: std::path::PathBuf,
    connector: Arc<dyn nocterm_ai::AgentConnector>,
    transfers: Arc<dyn nocterm_transfers::TransferRequests>,
    storage: nocterm_agent_runtime::AgentStorage,
) -> nocterm_agent::AgentServices {
    nocterm_agent::AgentServices {
        command_jobs: Arc::new(nocterm_agent_tools::JobsFactory),
        transfers,
        catalog: nocterm_agent_providers::catalog(),
        metadata: Arc::new(nocterm_agent_storage::NativeChatMetadata),
        connector,
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
