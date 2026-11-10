//! The hub ports a fixture composes the application over, and with a real hub adapter the
//! agent connector, chat history and agent hosts the application composes over it.
use std::sync::Arc;

/// The hub ports a fixture composes the application over.
#[derive(Default)]
pub(super) struct HubPorts {
    pub(super) deps: Option<nocterm_hubs::Deps>,
    pub(super) accounts: Option<Arc<dyn crate::hub_sources::HubAccounts>>,
    /// A real hub adapter: hub accounts, agents, chats and the agent connector are composed
    /// over it as the application composes them.
    pub(super) live: Option<Arc<nocterm_hub_io::HubLinks>>,
}

impl HubPorts {
    pub(super) fn live(links: Arc<nocterm_hub_io::HubLinks>) -> Self {
        Self {
            deps: Some(crate::startup::hub_dependencies(links.clone())),
            accounts: Some(links.clone()),
            live: Some(links),
        }
    }
}

impl From<Option<nocterm_hubs::Deps>> for HubPorts {
    fn from(deps: Option<nocterm_hubs::Deps>) -> Self {
        Self {
            deps,
            accounts: None,
            live: None,
        }
    }
}

/// The agent connector and chat history of the agent feature, and the hosts Agent access
/// writes through: over the real hub adapter when there is one, as the application composes
/// them; otherwise local agents and local history only.
pub(super) type AgentParts = (
    Arc<dyn nocterm_ai::AgentConnector>,
    Arc<dyn nocterm_ai::storage::ChatRepository>,
    Option<Arc<nocterm_ssh::authorized_keys::HostDirectory>>,
);

impl HubPorts {
    pub(super) fn agents(&self, paths: &nocterm_core::Paths) -> AgentParts {
        let local: Arc<dyn nocterm_ai::storage::ChatRepository> =
            Arc::new(nocterm_agent_storage::FileChatRepository::new(
                paths.state_dir().join("agent-chats"),
                Arc::new(nocterm_localfs::LocalFilesystem::new().unwrap()),
            ));
        let Some(links) = &self.live else {
            return (
                Arc::new(nocterm_acp::AcpConnector::unmanaged()),
                local,
                None,
            );
        };
        let hosts = Arc::new(nocterm_ssh::authorized_keys::HostDirectory::new(
            &nocterm_ssh::SshConfig::default(),
        ));
        (
            crate::startup::agent_connector(&std::env::current_exe().unwrap(), Some(links)),
            Arc::new(nocterm_hub_io::HubChatRepository::new(
                links,
                local,
                paths.state_dir().join("hub-chats"),
            )),
            Some(hosts),
        )
    }
}
