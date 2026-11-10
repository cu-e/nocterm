//! The account's agent key and the servers it is written to.

use crate::ids::ServerId;

/// The account's agent key: one Ed25519 key per account, without expiry. A server is
/// connected to agents when its `authorized_keys` holds [`AgentKey::authorized_line`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentKey {
    /// OpenSSH public key text.
    pub public_key: String,
    /// The whole `authorized_keys` line with its restrictions.
    pub authorized_line: String,
    /// `SHA256:…`.
    pub fingerprint: String,
}

impl AgentKey {
    /// Whether `line` of an `authorized_keys` file is this key's line, whatever options or
    /// comment it carries.
    pub fn is_line_of(&self, line: &str) -> bool {
        let key = key_blob(&self.public_key);
        !key.is_empty() && line.split_whitespace().any(|part| part == key)
    }
}

/// The base64 blob of an OpenSSH public key text (`ssh-ed25519 AAAA… comment`).
fn key_blob(public_key: &str) -> &str {
    public_key.split_whitespace().nth(1).unwrap_or_default()
}

/// What agents last found when they signed in to a server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ServerStatus {
    /// Not checked yet.
    #[default]
    Unchecked,
    /// The agent key signs in.
    Ready,
    /// The server refuses the agent key.
    NoAccess,
    /// The server presents another host key than the recorded one; agents do not connect.
    HostKeyChanged,
    /// The server could not be reached.
    Unreachable,
    /// The agent key was replaced after its line was written.
    KeyRotated,
}

impl ServerStatus {
    /// Short text for the server's row.
    pub fn label(self) -> &'static str {
        match self {
            Self::Unchecked => "Not checked",
            Self::Ready => "Agents ✓",
            Self::NoAccess => "No access — reconnect",
            Self::HostKeyChanged => "Host key changed",
            Self::Unreachable => "Unreachable",
            Self::KeyRotated => "Reconnect needed",
        }
    }

    /// Whether agents can sign in now.
    pub fn is_ready(self) -> bool {
        self == Self::Ready
    }
}

/// A server connected to the account's agents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubServer {
    pub id: ServerId,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub host_key_fingerprint: String,
    pub status: ServerStatus,
    /// Why the status is not ready, readable by a person; empty otherwise.
    pub status_message: String,
}

impl HubServer {
    /// Whether this record is the server `host:port` with `user`.
    pub fn is(&self, host: &str, port: u16, user: &str) -> bool {
        self.host.eq_ignore_ascii_case(host) && self.port == port && self.user == user
    }
}

/// A server whose `authorized_keys` already holds the agent key's line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewServer {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// The OpenSSH public host key this device recorded for the server.
    pub host_key: String,
}

/// What removing a server came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRemoval {
    /// The agent key's line was removed from the server too.
    pub line_removed: bool,
    /// Why the line stayed, readable by a person.
    pub message: String,
}
