//! Ports for reading and replacing the `authorized_keys` file of a server the user is
//! connected to, and the text rules for adding a line to it.
//!
//! The file is reached only through a session the user already opened: this crate holds no
//! connection. An adapter implements [`AuthorizedKeysFile`] over that session and tells
//! [`ConnectedHosts`] when a connection comes up.
use futures::future::BoxFuture;
use nocterm_core::error::SharedCause;
use nocterm_session::RemoteFs;
use std::sync::Arc;

/// The most the file may hold; a larger one is not touched.
pub const MAX_AUTHORIZED_KEYS_BYTES: usize = 1024 * 1024;

/// Who the user is on a server, as the trust registry records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostIdentity {
    /// The user whose `authorized_keys` is edited.
    pub user: String,
    /// The server host.
    pub host: String,
    /// SHA-256 fingerprint of the server's host key as this device recorded it.
    pub host_key_fingerprint: String,
}

/// Why the file could not be read or replaced.
#[derive(Clone, Debug, thiserror::Error)]
pub enum HostAccessError {
    /// The user has no open session to the connection.
    #[error("there is no open session to the server")]
    NotConnected,
    /// The file differs from what the change was computed from.
    #[error("the server's authorized_keys changed meanwhile")]
    Changed,
    /// The file is larger than this client edits.
    #[error("the server's authorized_keys is too large to edit")]
    TooLarge,
    /// The file is not text.
    #[error("the server's authorized_keys is not text")]
    NotText,
    /// The server refused or the session failed.
    #[error("the server could not read or change authorized_keys")]
    Host(#[source] SharedCause),
}

/// The `authorized_keys` file of one user on one connected server.
pub trait AuthorizedKeysFile: Send + Sync {
    /// The whole file, or the empty text when there is none.
    fn read(&self) -> BoxFuture<'static, Result<String, HostAccessError>>;
    /// Publishes `updated` atomically if the file still equals `previous`; every other line
    /// survives byte for byte and the permissions stay private.
    fn replace(
        &self,
        previous: String,
        updated: String,
    ) -> BoxFuture<'static, Result<(), HostAccessError>>;
}

/// An open way to a server's `authorized_keys`.
#[derive(Clone)]
pub struct HostAccess {
    /// Who the file belongs to.
    pub identity: HostIdentity,
    /// The file.
    pub file: Arc<dyn AuthorizedKeysFile>,
}

/// Finds the live access to a saved connection, if the user has a session to it now.
pub trait AgentHosts: Send + Sync {
    /// [`HostAccessError::NotConnected`] when no session of `connection` is up.
    fn access(&self, connection: &str) -> Result<HostAccess, HostAccessError>;
}

/// A saved connection that came up, offered with the file system of its session.
#[derive(Clone)]
pub struct ConnectedHost {
    /// The id of the saved connection.
    pub connection: String,
    /// Who the user is there.
    pub identity: HostIdentity,
    /// The session's file system.
    pub fs: Arc<dyn RemoteFs>,
}

/// Told about every connected saved connection, so the registry can be reconciled.
pub trait ConnectedHosts: Send + Sync {
    /// A session of `host.connection` is up. May be called repeatedly for the same session.
    fn connected(&self, host: ConnectedHost);
}

/// `file` with `line` appended on a line of its own, or unchanged if the line is already
/// there. A missing final newline is added first, so no other line is merged or altered.
pub fn with_line(file: &str, line: &str) -> String {
    if file.lines().any(|existing| existing == line) {
        return file.to_owned();
    }
    let mut updated = String::with_capacity(file.len() + line.len() + 2);
    updated.push_str(file);
    if !file.is_empty() && !file.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(line);
    updated.push('\n');
    updated
}
