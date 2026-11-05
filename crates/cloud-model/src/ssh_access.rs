//! Per-chat SSH access: a person's CA-signed binding of a chat to servers and the
//! short-lived user certificates devices issue for the runner's per-chat key (ADR 0145).
//!
//! The adapter verifies the binding's CA signature, its chat coordinate and each
//! certificate's CA signature before these values exist; this module decides what the
//! runner admits, when it asks for a certificate and when access ends. It holds public
//! values only: the subject's private key stays with its native custodian.
mod access;
mod parts;
mod ports;
#[cfg(test)]
mod tests;

pub use access::{BoundSelection, ChatSshAccess, KeyChange, SshAccessChange, SshCredential};
pub use parts::{BindingParts, IssuedCertificate, OpenRequest, SshAccessParts};
pub use ports::{
    BoundLogin, ChatSshCredentials, ChatSshKeys, SshLoginCredential, SshSubjectSeed,
    StoredSshAccess,
};

use crate::{
    authority::{AuthenticationBinding, TargetBinding},
    coordinator::{GrantClass, GrantReference},
    identity::{Fingerprint, Timestamp},
    payload::OpaqueBytes,
};

/// At most this many servers in one binding.
pub const MAX_BOUND_SERVERS: usize = 16;
/// A binding lasts at most 30 days.
pub const MAX_BINDING_WINDOW_MS: u64 = 30 * 24 * 60 * 60 * 1000;
/// A certificate request asks for 8 hours.
pub const DEFAULT_CERTIFICATE_VALIDITY_MS: u64 = 8 * 60 * 60 * 1000;
/// No certificate is accepted for longer than 24 hours.
pub const MAX_CERTIFICATE_VALIDITY_MS: u64 = 24 * 60 * 60 * 1000;
/// The largest certificate blob accepted.
pub const MAX_CERTIFICATE_BYTES: usize = 4096;
/// At most this many source addresses in a request.
pub const MAX_SOURCE_ADDRESSES: usize = 8;
/// The share of a certificate's life after which the runner asks for the next one.
pub const RENEW_AT_PERCENT: u64 = 80;

/// A refusal of an SSH binding or certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SshAccessError {
    #[error("the SSH binding or certificate is malformed")]
    Invalid,
    #[error("the SSH binding is not newer than the one already known")]
    Stale,
    #[error("the chat has no SSH binding")]
    NoBinding,
    #[error("the SSH grant is not the chat's current binding")]
    OtherGrant,
    #[error("the certificate answers no open request")]
    UnknownRequest,
    #[error("the certificate names another subject key")]
    SubjectKey,
    #[error("the certificate names other principals than the requested servers")]
    Principals,
    #[error("the certificate's validity is outside its request")]
    Validity,
}

/// One server of a binding: `SHA-256(profile id)[..16]`, as devices name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SshServerKey([u8; 16]);
impl SshServerKey {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Where the runner connects: host, port and user, compared exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshEndpoint {
    pub host: String,
    pub port: u16,
    pub user: String,
}
impl SshEndpoint {
    fn valid(&self) -> bool {
        !self.host.is_empty()
            && self.host.len() <= 253
            && self.port != 0
            && !self.user.is_empty()
            && self.user.len() <= 128
    }
}

/// The session access a device names to run a prompt on one bound server (contract ADR
/// 0011): fingerprints the adapter derives from the CA-signed binding, so a run selects the
/// server by equality and never constructs one from a prompt (ADR 0147).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundAccess {
    pub target: TargetBinding,
    pub authentication: AuthenticationBinding,
}

/// One server of a binding with its pinned host key and its per-server principal
/// (`nocterm-agent-<ca>-<server>`), which the adapter derives from the CA key. `access` is
/// `None` for a binding recorded before bound servers became session targets, or whose login
/// user is no valid principal: such a server can be certified but never selected by a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundTarget {
    pub server: SshServerKey,
    pub endpoint: SshEndpoint,
    pub host_key: Fingerprint,
    pub principal: String,
    pub access: Option<BoundAccess>,
}

/// A CA-signed binding admitted by the adapter for this exact chat. `signed` is the
/// binding's canonical signed bytes, echoed unchanged in every certificate request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshBindingOffer {
    grant: GrantReference,
    issued: Timestamp,
    expires: Timestamp,
    ca_key: [u8; 32],
    targets: Vec<BoundTarget>,
    signed: OpaqueBytes,
}
impl SshBindingOffer {
    pub fn new(
        grant: GrantReference,
        issued: Timestamp,
        expires: Timestamp,
        ca_key: [u8; 32],
        targets: Vec<BoundTarget>,
        signed: OpaqueBytes,
    ) -> Result<Self, SshAccessError> {
        let window = expires.elapsed_since(issued).unwrap_or(0);
        let unique = targets
            .iter()
            .enumerate()
            .all(|(index, target)| !targets[..index].iter().any(|t| t.server == target.server));
        if grant.class() != GrantClass::SshBinding
            || window == 0
            || window > MAX_BINDING_WINDOW_MS
            || targets.is_empty()
            || targets.len() > MAX_BOUND_SERVERS
            || !unique
            || targets
                .iter()
                .any(|t| !t.endpoint.valid() || t.principal.is_empty() || t.principal.len() > 128)
            || signed.is_empty()
        {
            return Err(SshAccessError::Invalid);
        }
        Ok(Self {
            grant,
            issued,
            expires,
            ca_key,
            targets,
            signed,
        })
    }
    pub fn grant(&self) -> GrantReference {
        self.grant
    }
    pub fn issued(&self) -> Timestamp {
        self.issued
    }
    pub fn expires(&self) -> Timestamp {
        self.expires
    }
    pub fn ca_key(&self) -> &[u8; 32] {
        &self.ca_key
    }
    pub fn targets(&self) -> &[BoundTarget] {
        &self.targets
    }
    pub fn signed(&self) -> &OpaqueBytes {
        &self.signed
    }
    fn target(&self, server: SshServerKey) -> Option<&BoundTarget> {
        self.targets.iter().find(|target| target.server == server)
    }
}

/// A device's answer to one certificate request: the OpenSSH certificate blob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshCertificateOffer {
    grant: GrantReference,
    request: [u8; 16],
    certificate: OpaqueBytes,
}
impl SshCertificateOffer {
    pub fn new(
        grant: GrantReference,
        request: [u8; 16],
        certificate: OpaqueBytes,
    ) -> Result<Self, SshAccessError> {
        if grant.class() != GrantClass::SshBinding
            || certificate.is_empty()
            || certificate.len() > MAX_CERTIFICATE_BYTES
        {
            return Err(SshAccessError::Invalid);
        }
        Ok(Self {
            grant,
            request,
            certificate,
        })
    }
    pub fn grant(&self) -> GrantReference {
        self.grant
    }
    pub fn request(&self) -> &[u8; 16] {
        &self.request
    }
    pub fn certificate(&self) -> &OpaqueBytes {
        &self.certificate
    }
}

/// What a certificate states, read by the adapter after it checked the CA signature
/// and the allowed restrictions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateFacts {
    pub subject_key: [u8; 32],
    pub principals: Vec<String>,
    pub valid_after: Timestamp,
    pub valid_before: Timestamp,
}

/// Reads a certificate blob under a CA key; implemented by the contract adapter.
pub trait SshCertificateReader: Send + Sync {
    fn read(
        &self,
        certificate: &[u8],
        ca_key: &[u8; 32],
    ) -> Result<CertificateFacts, SshAccessError>;
}

/// The servers this runner can reach and the addresses servers see it connect from.
/// Every target carries its host key, which must equal the binding's: a managed boot
/// target without one is not listed (ADR 0147).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshReach {
    endpoints: Vec<(SshEndpoint, Fingerprint)>,
    egress: Vec<String>,
}
impl SshReach {
    pub fn new(
        endpoints: Vec<(SshEndpoint, Fingerprint)>,
        egress: Vec<String>,
    ) -> Result<Self, SshAccessError> {
        if endpoints.iter().any(|(endpoint, _)| !endpoint.valid())
            || endpoints.len() > 256
            || egress.len() > MAX_SOURCE_ADDRESSES
            || egress.iter().any(|address| {
                address.is_empty()
                    || address.len() > 64
                    || !address
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() || b".:/".contains(&byte))
            })
        {
            return Err(SshAccessError::Invalid);
        }
        Ok(Self { endpoints, egress })
    }
    pub fn egress(&self) -> &[String] {
        &self.egress
    }
    /// Whether this runner may connect to `target` ("select, never construct").
    pub fn admits(&self, target: &BoundTarget) -> SshServerReach {
        let known = self
            .endpoints
            .iter()
            .any(|(endpoint, key)| endpoint == &target.endpoint && key == &target.host_key);
        if known {
            SshServerReach::Admitted
        } else {
            SshServerReach::Unreachable
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshServerReach {
    Admitted,
    Unreachable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshBindingRefusal {
    /// The runner states no source address, so no certificate can be bound to it.
    NoSourceAddress,
    /// No server of the binding is one this runner may reach.
    NoReachableServer,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshAccessPhase {
    /// Waiting for a device to answer a certificate request.
    Pending,
    Active {
        request: [u8; 16],
        valid_before: Timestamp,
    },
    Refused(SshBindingRefusal),
    Revoked,
    Expired,
}

/// The chat event that tells devices where a binding stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshAccessReport {
    pub grant: GrantReference,
    pub access: SshAccessPhase,
    pub servers: Vec<(SshServerKey, SshServerReach)>,
}

/// The chat event asking any account device for a certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshCertificateRequest {
    pub request: [u8; 16],
    pub binding: OpaqueBytes,
    pub subject_key: [u8; 32],
    pub source_addresses: Vec<String>,
    pub servers: Vec<SshServerKey>,
    pub not_after: Timestamp,
}

/// One chat event of SSH access, journaled and published like every other event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshAccessEvent {
    Requested(Box<SshCertificateRequest>),
    State(SshAccessReport),
}
