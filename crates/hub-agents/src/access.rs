//! Agent access of a saved server: the account's agent key line in the server's
//! `authorized_keys`, written through the session the user opened, and the hub's record of the
//! server (ADR 0148).

use std::sync::Arc;

use nocterm_hub_agents_model::{
    HubAgentsError, HubServer, HubServers, NewServer, ServerId, ServerRemoval,
};
use nocterm_hubs_model::HubId;
use nocterm_ssh_keys::authorized_keys::{AgentHosts, HostAccessError, with_line, without_lines};

/// The saved connection whose agent access changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentTarget {
    /// The id of the saved connection, as its session announced it.
    pub connection: String,
    /// The name the hub shows for the server.
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
}

/// Why agent access could not be changed.
#[derive(Debug, thiserror::Error)]
pub enum AgentAccessError {
    /// No session to the server is open, so its `authorized_keys` cannot be written.
    #[error("no session to the server is open")]
    NotConnected,
    /// This device recorded no host key for the server, so the hub could not check it.
    #[error("no host key is recorded for the server")]
    UnknownHostKey,
    /// The server's `authorized_keys` could not be read or replaced.
    #[error("the server's authorized_keys could not be edited")]
    Host(#[source] HostAccessError),
    /// The hub refused or could not be reached.
    #[error(transparent)]
    Hub(#[from] HubAgentsError),
}

impl From<HostAccessError> for AgentAccessError {
    fn from(error: HostAccessError) -> Self {
        match error {
            HostAccessError::NotConnected => Self::NotConnected,
            error => Self::Host(error),
        }
    }
}

/// Writes the agent key's line to the server, then records the server on the hub with the host
/// key this device recorded for it.
pub(crate) async fn grant(
    servers: Arc<dyn HubServers>,
    hosts: Arc<dyn AgentHosts>,
    hub: HubId,
    target: AgentTarget,
) -> Result<HubServer, AgentAccessError> {
    let access = hosts.access(&target.connection)?;
    if access.identity.host_key.is_empty() {
        return Err(AgentAccessError::UnknownHostKey);
    }
    let key = servers.agent_key(&hub).await?;
    let current = access.file.read().await?;
    let updated = with_line(&current, &key.authorized_line);
    if updated != current {
        access.file.replace(current, updated).await?;
    }
    let server = NewServer {
        name: target.name,
        host: target.host,
        port: target.port,
        user: access.identity.user,
        host_key: access.identity.host_key,
    };
    Ok(servers.add_server(&hub, server).await?)
}

/// Removes the agent key's line while a session to the server is open, then the hub's record;
/// without a session the hub is asked to remove the line itself.
pub(crate) async fn withdraw(
    servers: Arc<dyn HubServers>,
    hosts: Arc<dyn AgentHosts>,
    hub: HubId,
    connection: String,
    server: ServerId,
) -> Result<ServerRemoval, AgentAccessError> {
    match hosts.access(&connection) {
        Ok(access) => {
            let key = servers.agent_key(&hub).await?;
            let current = access.file.read().await?;
            let updated = without_lines(&current, |line| key.is_line_of(line));
            if updated != current {
                access.file.replace(current, updated).await?;
            }
        }
        Err(HostAccessError::NotConnected) => {}
        Err(error) => return Err(AgentAccessError::Host(error)),
    }
    Ok(servers.remove_server(&hub, &server).await?)
}
