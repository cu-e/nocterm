//! Captured local-history partition and validity; no Cloud authority is inferred.
use super::{AccountScope, Fingerprint, NamespaceId};
use crate::HubId;
use serde::{Deserialize, Serialize};

/// Old hubs have no authenticated stable instance authority. Their local history
/// is separated by retained configuration, authenticated account and account key.
/// Address aliases may have distinct partitions; this is never a Cloud identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyConfiguredAccount {
    hub: HubId,
    account: NamespaceId,
    key: Fingerprint,
}
impl LegacyConfiguredAccount {
    pub fn new(hub: HubId, account: NamespaceId, key: Fingerprint) -> Self {
        Self { hub, account, key }
    }
    pub fn hub(&self) -> &HubId {
        &self.hub
    }
    pub fn account(&self) -> &NamespaceId {
        &self.account
    }
    pub fn key(&self) -> Fingerprint {
        self.key
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "identity",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum LocalAccountPartition {
    Stable(AccountScope),
    LegacyConfigured(LegacyConfiguredAccount),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LocalAccountSourceError {
    #[error("the captured hub account is no longer signed in or unlocked")]
    Stale,
    #[error("the captured hub account identity is unavailable")]
    Unavailable,
}

/// Authenticated adapter-owned capture. A plain partition constructor is not
/// evidence of authentication. Ordinary token refresh retains the source;
/// sign-out, replacement login, key rotation and lock invalidate the old capture.
pub trait LocalAccountSource: Send + Sync {
    fn partition(&self) -> LocalAccountPartition;
    fn check_current(&self) -> Result<(), LocalAccountSourceError>;
}

/// Captures only a new attempt; existing conversations retain their original source.
/// Unlock/sign-in can make later captures available without rewriting old handles.
pub trait LocalAccountSourceFactory: Send + Sync {
    fn capture(&self) -> Result<std::sync::Arc<dyn LocalAccountSource>, crate::HubError>;
}
