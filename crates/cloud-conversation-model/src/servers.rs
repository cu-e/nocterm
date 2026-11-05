//! The servers a Cloud chat is bound to and the ones it can be bound to (ADR 0147). Plain
//! display data: binding goes through the port, and a server here grants nothing.
use core::fmt;

/// The longest binding: thirty days.
pub const MAX_BINDING_WINDOW: std::time::Duration = std::time::Duration::from_secs(30 * 86_400);
/// The binding a person gets without choosing: seven days.
pub const DEFAULT_BINDING_WINDOW: std::time::Duration = std::time::Duration::from_secs(7 * 86_400);

/// One server of the account's agent CA registry: `SHA-256(saved connection id)[..16]`, the
/// id a binding and its certificates name it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerKey([u8; 16]);
impl ServerKey {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}
impl fmt::Display for ServerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

/// Where one bound server stands for the chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerAccess {
    /// The runner may reach it and waits for an account device to certify its key.
    Pending,
    /// A certificate lets the runner sign in until `until_ms`.
    Active { until_ms: i64 },
    /// The runner cannot reach this server.
    Unreachable,
    /// The runner refused the binding (no egress, no reachable server, or invalid).
    Refused,
    /// The binding's window ended.
    Expired,
    /// The binding was revoked.
    Revoked,
}

/// One server of the chat's current binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundServer {
    pub server: ServerKey,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub access: ServerAccess,
    /// A prompt may name it now ([`crate::ConversationAction::Prompt`]'s `server`): the runner
    /// admitted it and the binding window is open.
    pub selectable: bool,
}

/// The chat's current binding as the verified history states it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerBinding {
    /// The bound servers, in the binding's order; empty when the chat is not bound.
    pub servers: Vec<BoundServer>,
    /// When the binding ends, once the runner echoed it.
    pub expires_at_ms: Option<i64>,
}
impl ServerBinding {
    pub fn is_bound(&self) -> bool {
        !self.servers.is_empty()
    }
    /// The bound servers a prompt may name now.
    pub fn selectable(&self) -> impl Iterator<Item = &BoundServer> {
        self.servers.iter().filter(|server| server.selectable)
    }
}

/// A server of the account's CA registry the chat can be bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerChoice {
    pub server: ServerKey,
    /// The saved connection it was trusted through.
    pub connection: String,
    pub host: String,
    pub user: String,
    /// The SSH port of the saved connection, once this device connected to it; a server
    /// without one cannot be bound yet.
    pub port: Option<u16>,
    /// It still holds the shared CA principal: certificates open it only after its next
    /// connection gives it its own line.
    pub needs_ca_update: bool,
}

/// Why binding servers or naming one in a prompt was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerRefusal {
    /// The account has no agent CA yet.
    NoCa,
    /// A chosen server is not trusted under the account's current CA.
    Untrusted,
    /// A chosen server's port is not known yet.
    UnknownPort,
    /// No server, or more than a binding takes, was chosen.
    Servers,
    /// The window is empty or longer than thirty days.
    Window,
    /// The chat is not bound to servers.
    NotBound,
    /// The named server is not one a prompt may run on now.
    NotSelectable,
}
impl fmt::Display for ServerRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoCa => "the account has no agent CA yet",
            Self::Untrusted => "a chosen server is not trusted by the account's CA",
            Self::UnknownPort => "connect to the server once so its port is known",
            Self::Servers => "choose between one and sixteen servers",
            Self::Window => "a binding lasts at most thirty days",
            Self::NotBound => "the chat is not bound to servers",
            Self::NotSelectable => "the server cannot run prompts now",
        })
    }
}
