//! Native-free application boot fixtures own their temporary directory lifetime.
use super::{Arc, MockTransport};

struct FixtureDirectory {
    _directory: tempfile::TempDir,
}
impl gpui_kit::Global for FixtureDirectory {}

/// Installs the application's globals as the application does, with the
/// state in `directory` and nothing persisted.
pub(super) fn boot(
    directory: tempfile::TempDir,
    settings: nocterm_settings::SettingsDocument,
    transport: Arc<MockTransport>,
    vault: bool,
    cx: &mut gpui_kit::App,
) -> crate::bootstrap::Booted {
    boot_with_files(directory, settings, transport, vault, None, cx)
}

pub(super) fn boot_with_files(
    directory: tempfile::TempDir,
    settings: nocterm_settings::SettingsDocument,
    transport: Arc<MockTransport>,
    vault: bool,
    files: Option<nocterm_files::Deps>,
    cx: &mut gpui_kit::App,
) -> crate::bootstrap::Booted {
    boot_with_ports(
        directory,
        nocterm_store::SettingsConfig::in_memory(settings),
        (
            transport,
            nocterm_terminal::LocalSessions::disabled("/bin/bash".into()),
        ),
        vault,
        files,
        None,
        cx,
    )
}
pub(super) fn boot_with_ports(
    directory: tempfile::TempDir,
    settings: nocterm_store::SettingsConfig,
    terminal: (Arc<MockTransport>, nocterm_terminal::LocalSessions),
    vault: bool,
    files: Option<nocterm_files::Deps>,
    themes: Option<crate::bootstrap::Themes>,
    cx: &mut gpui_kit::App,
) -> crate::bootstrap::Booted {
    boot_with_ports_and_probe(
        directory,
        settings,
        terminal,
        vault,
        files,
        themes,
        Arc::new(nocterm_ai::testing::ScriptedSandboxProbe::new(
            nocterm_ai::sandbox::Availability::Unsupported,
        )),
        cx,
    )
}
pub(super) fn boot_with_ports_and_probe(
    directory: tempfile::TempDir,
    settings: nocterm_store::SettingsConfig,
    terminal: (Arc<MockTransport>, nocterm_terminal::LocalSessions),
    vault: bool,
    files: Option<nocterm_files::Deps>,
    themes: Option<crate::bootstrap::Themes>,
    probe: Arc<dyn nocterm_ai::sandbox::SandboxProbe>,
    cx: &mut gpui_kit::App,
) -> crate::bootstrap::Booted {
    let (transport, local) = terminal;
    let local_host = local.is_available().then(|| {
        Arc::new(nocterm_session::testing::FakeExec::default())
            as Arc<dyn nocterm_session::HostExec>
    });
    let paths = nocterm_core::Paths::rooted_at(directory.path());
    let vault = vault.then(|| crate::bootstrap::VaultSetup {
        file: directory.path().join("vault.bin"),
        device_unlock: false,
    });
    let files = files.unwrap_or_else(|| fixture_files(directory.path()));
    cx.set_global(FixtureDirectory {
        _directory: directory,
    });
    let retention = nocterm_ai::model::retention::RetentionAuthority::new(
        nocterm_ai::history::MAX_RESIDENT_BYTES,
    )
    .unwrap();
    let booted = crate::bootstrap::bootstrap(
        crate::bootstrap::Services {
            images: {
                let codec = Arc::new(nocterm_ai::testing::ScriptedImageCodec::default());
                nocterm_agent::ImageDeps::new(codec.clone(), codec)
            },
            tokens: nocterm_ui::DesignTokens::builtin(),
            settings,
            persist: false,
            connections: crate::connection_setup::memory(),
            transport,
            local,
            local_host,
            terminal_auth_factory: None,
            files,
            themes,
            agent: nocterm_agent::AgentServices {
                retention: retention.clone(),
                sandbox_probe: probe,
                metadata: Arc::new(nocterm_ai::testing::ScriptedChatMetadata::default()),
                connector: Arc::new(nocterm_acp::AcpConnector::unmanaged()),
                bridge: Arc::new(nocterm_acp::BridgeServer::new(
                    paths.clone(),
                    nocterm_acp::RelayCommand {
                        program: std::env::current_exe().expect("test executable"),
                        args: vec![crate::AGENT_BRIDGE.into()],
                    },
                )),
                storage: futures::executor::block_on(nocterm_agent_runtime::AgentStorage::prepare(
                    Arc::new(nocterm_agent_storage::FileAgentStateRepository::new(
                        paths.state_dir().join("agents.toml"),
                    )),
                    Arc::new(nocterm_agent_storage::FileChatRepository::new(
                        paths.state_dir().join("agent-chats"),
                    )),
                )),
                workdir_resolver: Arc::new(nocterm_acp::NativeWorkdirResolver::default()),
                usage_source: Arc::new(nocterm_ai::testing::ScriptedUsageSource::default()),
                workdir: paths.state_dir().join("agent-workspace"),
                terminal_auth: None,
                private_dirs: Vec::new(),
                shared_dirs: Vec::new(),
            },
            paths,
            vault,
        },
        cx,
    )
    .expect("valid fixture settings catalogs");
    cx.set_reduce_motion(true);
    booted
}

fn fixture_files(directory: &std::path::Path) -> nocterm_files::Deps {
    let local = Arc::new(nocterm_session::testing::InMemoryFs::default());
    seed_native_ancestors(&local, directory);
    let transfers = nocterm_transfer_runtime::TransferRuntime::start(
        nocterm_transfer_runtime::TransferRuntimeDeps::new(local.clone(), Default::default()),
    )
    .unwrap();
    nocterm_files::Deps::new(
        local.clone(),
        local,
        transfers,
        Arc::new(nocterm_session::testing::FakeExec::default()),
    )
}
pub(super) fn seed_native_ancestors(
    fs: &nocterm_session::testing::InMemoryFs,
    directory: &std::path::Path,
) {
    for ancestor in directory.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if ancestor.parent().is_some() {
            fs.seed_native_directory(ancestor).unwrap();
        }
    }
}
