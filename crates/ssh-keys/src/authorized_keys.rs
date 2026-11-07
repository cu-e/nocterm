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

/// Who the user is on a server, and the server's host key as this device recorded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostIdentity {
    /// The user whose `authorized_keys` is edited.
    pub user: String,
    /// The server host.
    pub host: String,
    /// The OpenSSH public host key this device recorded for the server; empty when none is.
    pub host_key: String,
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
    /// A cooperating edit owns the fixed remote lock; interrupted edits require manual recovery.
    #[error("authorized_keys editing is locked at {path}")]
    EditLocked { path: String },
    /// The server cannot support the safe automatic editing protocol.
    #[error("the server cannot safely edit authorized_keys automatically")]
    Unsupported(#[source] SharedCause),
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
    /// Compares with `previous` under an adapter's cooperative edit lock and reports
    /// observed changes. Unrelated lines survive byte for byte. Interrupted edits may
    /// require explicit lock recovery. SFTP cannot atomically compare and swap against
    /// uncooperative external writers; callers must not claim that stronger guarantee.
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
    /// Who the user is there. An empty host key is filled in by the adapter from the keys
    /// this device recorded for the host.
    pub identity: HostIdentity,
    /// The SSH port, which tells the recorded host keys apart.
    pub port: u16,
    /// The session's file system.
    pub fs: Arc<dyn RemoteFs>,
}

/// Told about every connected saved connection, so its `authorized_keys` can be reached.
pub trait ConnectedHosts: Send + Sync {
    /// Enriches immutable announcement metadata on a background executor. Does not
    /// publish session state; composition must recheck the original session before
    /// delivering the prepared announcement to `connected`.
    fn prepare(&self, host: ConnectedHost) -> ConnectedHost {
        host
    }
    /// Publishes a still-current prepared session without native I/O. May be repeated.
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

/// `file` without the lines `is_line` picks; every other line stays byte for byte.
pub fn without_lines(file: &str, is_line: impl Fn(&str) -> bool) -> String {
    file.split_inclusive('\n')
        .filter(|line| !is_line(line.trim_end_matches(['\n', '\r'])))
        .collect()
}
