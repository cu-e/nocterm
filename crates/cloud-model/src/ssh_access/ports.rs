//! Custody of a chat's SSH subject key and what a session receives from it.
use super::{SshAccessParts, SshEndpoint};
use crate::{
    authority::{AuthenticationBinding, StreamBinding, TargetBinding},
    identity::{Fingerprint, Timestamp},
    journal::JournalFuture,
    payload::OpaqueBytes,
};
use std::fmt;
use zeroize::Zeroizing;

/// The seed of a chat's Ed25519 subject key. It never formats itself.
#[derive(Clone)]
pub struct SshSubjectSeed(Zeroizing<[u8; 32]>);
impl SshSubjectSeed {
    pub fn new(seed: [u8; 32]) -> Self {
        Self(Zeroizing::new(seed))
    }
    pub fn expose(&self) -> &[u8; 32] {
        &self.0
    }
    /// The raw public key a certificate names as its subject.
    pub fn public_key(&self) -> [u8; 32] {
        nocterm_ssh_keys::ed25519_public_key(&self.0)
    }
}
impl fmt::Debug for SshSubjectSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SshSubjectSeed(..)")
    }
}

/// One chat's stored SSH row: the subject seed while a binding needs it, and the access.
#[derive(Clone, Debug)]
pub struct StoredSshAccess {
    pub seed: Option<SshSubjectSeed>,
    pub access: SshAccessParts,
}

/// The runner journal's per-chat SSH rows (journal v8). Implementations seal each row
/// with the journal cipher of its chat, so the seed is never stored in the clear.
pub trait ChatSshKeys: Send + Sync {
    fn ssh_access(&self, stream: StreamBinding) -> JournalFuture<Option<StoredSshAccess>>;
    fn store_ssh_access(&self, stream: StreamBinding, stored: StoredSshAccess)
    -> JournalFuture<()>;
    /// The chats that hold a row, for the renewal sweep after a restart.
    fn ssh_streams(&self) -> JournalFuture<Vec<StreamBinding>>;
}

/// What a session needs to sign in to one bound server: the chat's key, its certificate
/// and the principal the certificate names for that server.
#[derive(Clone, Debug)]
pub struct SshLoginCredential {
    pub seed: SshSubjectSeed,
    pub certificate: OpaqueBytes,
    pub principal: String,
    pub valid_before: Timestamp,
}

/// A bound server a run selected by its session access (ADR 0147): where to connect, the
/// host key to pin and, while a certificate is active for it, the login.
#[derive(Clone, Debug)]
pub struct BoundLogin {
    pub endpoint: SshEndpoint,
    pub host_key: Fingerprint,
    /// `None` while no certificate is active: the session waits for a device.
    pub credential: Option<SshLoginCredential>,
}

/// The session broker's view of the chats' active certificates.
pub trait ChatSshCredentials: Send + Sync {
    fn credential(
        &self,
        stream: &StreamBinding,
        endpoint: &SshEndpoint,
        host_key: &Fingerprint,
        now: Timestamp,
    ) -> Option<SshLoginCredential>;
    /// The admitted server of the chat's current binding whose session access is exactly
    /// `target` and `authentication`, inside the binding's window; `None` selects nothing.
    fn bound(
        &self,
        stream: &StreamBinding,
        target: &TargetBinding,
        authentication: &AuthenticationBinding,
        now: Timestamp,
    ) -> Option<BoundLogin>;
}
