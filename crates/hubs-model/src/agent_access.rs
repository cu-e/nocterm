//! What the user lets cloud agents do on their servers: the CA that signs the agents'
//! certificates, the servers that trust it, and the chat accesses that are active.

use futures::future::BoxFuture;
use nocterm_core::error::SharedCause;

use crate::address::HubId;
use crate::error::HubError;

/// The personal CA of the account, as the user compares and copies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCa {
    /// `SHA256:...` fingerprint of the CA public key.
    pub fingerprint: String,
    /// When this CA was created, in milliseconds since the Unix epoch.
    pub created_at_ms: i64,
}

/// Where a server stands with the CA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerTrust {
    /// The server trusts the current CA.
    Trusted,
    /// The user denied access but the server was not reachable yet; it is removed on the next
    /// connection.
    RemovalPending,
    /// The CA line was removed, as the user asked.
    Removed,
    /// The CA line vanished or names an outdated CA without the user asking for it.
    Drifted,
}

/// One server that trusts, or trusted, the CA.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedServer {
    /// The saved connection through which the server is reached.
    pub connection: String,
    /// The user whose `authorized_keys` holds the line.
    pub user: String,
    /// The server host.
    pub host: String,
    /// Where the server stands.
    pub state: ServerTrust,
    /// The server trusts the CA with the shared principal of an earlier client: a chat
    /// certificate opens it only after its next connection gives it its own line.
    pub needs_ca_update: bool,
    /// When the line was added, in milliseconds since the Unix epoch.
    pub added_at_ms: i64,
    /// The device that added it.
    pub added_by_device: String,
    /// When `authorized_keys` was last read, in milliseconds since the Unix epoch.
    pub last_verified_at_ms: Option<i64>,
}

/// The CA of the account and the servers that trust it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentTrustOverview {
    /// `None` until the user allows a first server.
    pub ca: Option<AgentCa>,
    /// Every server of the registry, removed ones included.
    pub servers: Vec<TrustedServer>,
}

/// A server the user is connected to now, which the CA could be allowed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostChoice {
    /// The saved connection.
    pub connection: String,
    /// The user of the session.
    pub user: String,
    /// The server host.
    pub host: String,
}

/// One chat access an agent holds on the account's servers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentAccessGrant {
    /// The hub's identifier of the access.
    pub grant_id: String,
    /// The agent the access belongs to.
    pub agent_id: String,
    /// The chat, as hexadecimal text.
    pub chat_id: String,
    /// The servers the access reaches, as the hub lists them.
    pub targets: Vec<String>,
    /// When it was issued, in milliseconds since the Unix epoch.
    pub issued_at_ms: i64,
    /// When it expires, in milliseconds since the Unix epoch.
    pub expires_at_ms: i64,
}

/// Why an agent access operation failed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum AgentAccessError {
    #[error(transparent)]
    Hub(#[from] HubError),
    /// No session to the connection's server is open, so its `authorized_keys` cannot be read.
    #[error("the server of {connection} is not connected")]
    NotConnected { connection: String },
    /// `authorized_keys` changed while it was edited; nothing was written.
    #[error("the server's authorized_keys changed meanwhile")]
    HostChanged,
    /// `authorized_keys` cannot be edited as text, or is too large.
    #[error("the server's authorized_keys cannot be edited")]
    HostUnsupported,
    /// The server could not be reached or refused the edit.
    #[error("the server could not be edited")]
    Host {
        #[source]
        cause: SharedCause,
    },
    /// No server of the registry is reached through this connection.
    #[error("no server is registered for {connection}")]
    Unknown { connection: String },
    /// The account's records could not be read, decoded or written.
    #[error("the agent access records are unusable")]
    Records {
        #[source]
        cause: SharedCause,
    },
}

/// A result that completes later.
pub type AgentAccessFuture<T> = BoxFuture<'static, Result<T, AgentAccessError>>;

/// The servers that trust the account's agent CA: lists them, allows and denies them, and
/// keeps them in line with what their `authorized_keys` files really say.
///
/// A change to a server needs a session the user already opened to it; the registry is the
/// account's own encrypted collection, so a hub never reads it.
pub trait AgentHostTrust: Send + Sync {
    /// The servers with an open session, where [`AgentHostTrust::allow`] can work now.
    fn connected_hosts(&self) -> Vec<HostChoice>;

    /// The CA and the registry of `hub`'s account.
    fn overview(&self, hub: &HubId) -> AgentAccessFuture<AgentTrustOverview>;

    /// Lets the CA sign in to the server of `connection`: adds its line to `authorized_keys`
    /// and records the server. Creates the CA on the first call.
    fn allow(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<()>;

    /// Stops trusting the server of `connection`. A server that cannot be reached stays
    /// [`ServerTrust::RemovalPending`] and loses the line on its next connection; the state
    /// reached is returned.
    fn deny(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<ServerTrust>;

    /// Compares the server's `authorized_keys` with the registry after the user connected,
    /// finishes a pending removal and marks a line that vanished as drifted.
    fn reconcile(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<ServerTrust>;

    /// Replaces the CA with a new one and moves the servers that can be reached to it.
    fn reissue_ca(&self, hub: &HubId) -> AgentAccessFuture<AgentCa>;

    /// The `cert-authority` line of the server reached through `connection`, for the user
    /// to paste into it by hand; it admits only that server's principal.
    fn ca_line(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<String>;
}

/// The chat accesses agents hold, as the user lists and revokes them.
pub trait AgentGrants: Send + Sync {
    /// The accesses that are active on `hub`.
    fn list(&self, hub: &HubId) -> AgentAccessFuture<Vec<AgentAccessGrant>>;

    /// Ends the access `grant_id` at once.
    fn revoke(&self, hub: &HubId, grant_id: &str) -> AgentAccessFuture<()>;
}
