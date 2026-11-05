//! The displayed recovery key retains its original recipient and acknowledgement.
use crate::{HubError, HubFuture, HubId};
use nocterm_session::Secret;
use std::{fmt, sync::Arc};

/// Native authority for acknowledging exactly one displayed recovery key.
pub trait RecoveryKeyConfirmation: Send + Sync {
    fn check_current(&self) -> Result<(), HubError>;
    fn confirm(&self) -> HubFuture<()>;
}

#[derive(Clone)]
pub struct RecoveryKeyOffer {
    hub: HubId,
    key: Secret,
    confirmation: Arc<dyn RecoveryKeyConfirmation>,
}
impl RecoveryKeyOffer {
    /// The injected capability establishes authority; this wrapper does not authenticate it.
    pub fn new(hub: HubId, key: Secret, confirmation: Arc<dyn RecoveryKeyConfirmation>) -> Self {
        Self {
            hub,
            key,
            confirmation,
        }
    }
    pub fn hub(&self) -> &HubId {
        &self.hub
    }
    pub fn key(&self) -> &Secret {
        &self.key
    }
    pub fn confirm(&self) -> HubFuture<()> {
        self.confirmation.confirm()
    }
    pub fn check_current(&self) -> Result<(), HubError> {
        self.confirmation.check_current()
    }
}
impl fmt::Debug for RecoveryKeyOffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecoveryKeyOffer").finish_non_exhaustive()
    }
}
impl PartialEq for RecoveryKeyOffer {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.confirmation, &other.confirmation)
    }
}

/// Native authority to replace the recovery envelope of one original open owner.
pub trait RecoveryKeyIssuer: Send + Sync {
    fn check_current(&self) -> Result<(), HubError>;
    fn issue(&self) -> HubFuture<RecoveryKeyOffer>;
}

#[derive(Clone)]
pub struct RecoveryKeyReissue {
    hub: HubId,
    issuer: Arc<dyn RecoveryKeyIssuer>,
}
impl RecoveryKeyReissue {
    pub fn new(hub: HubId, issuer: Arc<dyn RecoveryKeyIssuer>) -> Self {
        Self { hub, issuer }
    }
    pub fn hub(&self) -> &HubId {
        &self.hub
    }
    pub fn issue(&self) -> HubFuture<RecoveryKeyOffer> {
        self.issuer.issue()
    }
    pub fn check_current(&self) -> Result<(), HubError> {
        self.issuer.check_current()
    }
}
impl fmt::Debug for RecoveryKeyReissue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecoveryKeyReissue").finish_non_exhaustive()
    }
}
impl PartialEq for RecoveryKeyReissue {
    fn eq(&self, other: &Self) -> bool {
        self.hub == other.hub && Arc::ptr_eq(&self.issuer, &other.issuer)
    }
}
