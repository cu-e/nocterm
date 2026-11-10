//! Captured descriptive source identity shared by chat, storage and cloud owners.
//! Constructors check shape only; trusted adapters authenticate the supplied values.
use serde::{Deserialize, Serialize};
use std::fmt;
mod local;
pub use local::{
    LegacyConfiguredAccount, LocalAccountPartition, LocalAccountSource, LocalAccountSourceError,
    LocalAccountSourceFactory,
};

pub const MAX_NAMESPACE_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceIdentityError {
    #[error("an opaque namespace identifier must contain 1 to 128 UTF-8 bytes")]
    NamespaceLength,
}

/// Supplied digest/fingerprint bytes, not a key validator or signature verifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Fingerprint([u8; 32]);
impl Fingerprint {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NamespaceId(String);
impl NamespaceId {
    pub fn new(value: impl Into<String>) -> Result<Self, SourceIdentityError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_NAMESPACE_BYTES {
            return Err(SourceIdentityError::NamespaceLength);
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for NamespaceId {
    type Error = SourceIdentityError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<NamespaceId> for String {
    fn from(value: NamespaceId) -> Self {
        value.0
    }
}
impl fmt::Debug for NamespaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NamespaceId").field(&self.0.len()).finish()
    }
}

/// An instance label bound to its originally authenticated authority, not its address.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubIdentity {
    instance: NamespaceId,
    authority: Fingerprint,
}
impl HubIdentity {
    pub fn new(instance: NamespaceId, authority: Fingerprint) -> Self {
        Self {
            instance,
            authority,
        }
    }
    pub fn instance(&self) -> &NamespaceId {
        &self.instance
    }
    pub fn authority(&self) -> Fingerprint {
        self.authority
    }
}

/// Captured account ID and expected account-key identity from the original installation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountIdentity {
    id: NamespaceId,
    key: Fingerprint,
}
impl AccountIdentity {
    pub fn new(id: NamespaceId, key: Fingerprint) -> Self {
        Self { id, key }
    }
    pub fn id(&self) -> &NamespaceId {
        &self.id
    }
    pub fn key(&self) -> Fingerprint {
        self.key
    }
}

/// A descriptive account partition. Construction is not account authentication.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountScope {
    hub: HubIdentity,
    account: AccountIdentity,
}
impl AccountScope {
    pub fn new(hub: HubIdentity, account: AccountIdentity) -> Self {
        Self { hub, account }
    }
    pub fn hub(&self) -> &HubIdentity {
        &self.hub
    }
    pub fn account(&self) -> &AccountIdentity {
        &self.account
    }
}
