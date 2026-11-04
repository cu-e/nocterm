//! Plain stored form of a chat's SSH access; the runner journal owns its codec.
use super::{SshBindingOffer, SshBindingRefusal, SshServerKey, SshServerReach};
use crate::{
    identity::{GrantEpoch, GrantId, Timestamp},
    payload::OpaqueBytes,
};

/// A certificate request the runner published and no device answered yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub id: [u8; 16],
    pub servers: Vec<SshServerKey>,
    pub at: Timestamp,
    pub not_after: Timestamp,
}

/// The certificate a device issued for an open request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuedCertificate {
    pub request: [u8; 16],
    pub servers: Vec<SshServerKey>,
    pub blob: OpaqueBytes,
    pub valid_after: Timestamp,
    pub valid_before: Timestamp,
}

/// The chat's current binding and how far it got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingParts {
    pub offer: SshBindingOffer,
    pub reach: Vec<(SshServerKey, SshServerReach)>,
    pub refused: Option<SshBindingRefusal>,
    pub request: Option<OpenRequest>,
    pub certificate: Option<IssuedCertificate>,
}

/// Everything `ChatSshAccess` keeps: the newest epoch seen per grant id (replays of
/// older bindings stay refused after an unbind) and the current binding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SshAccessParts {
    pub epochs: Vec<(GrantId, GrantEpoch)>,
    pub binding: Option<BindingParts>,
}
