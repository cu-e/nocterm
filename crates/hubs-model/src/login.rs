//! Conditional cleanup of the original login after a consumer's delayed publication.
use crate::{HubError, HubId};
use std::{fmt, sync::Arc};

/// The adapter retains native membership and local file-login ownership.
pub trait LoginOwner: Send + Sync {
    fn check_current(&self) -> Result<(), HubError>;
    fn discard(&self) -> Result<(), HubError>;
}
/// A successful login's original owner, independent from the vault key lifecycle.
#[derive(Clone)]
pub struct LoginReceipt {
    hub: HubId,
    owner: Arc<dyn LoginOwner>,
}
impl LoginReceipt {
    pub fn new(hub: HubId, owner: Arc<dyn LoginOwner>) -> Self {
        Self { hub, owner }
    }
    pub fn hub(&self) -> &HubId {
        &self.hub
    }
    pub fn check_current(&self) -> Result<(), HubError> {
        self.owner.check_current()
    }
    pub fn discard(&self) -> Result<(), HubError> {
        self.owner.discard()
    }
}
impl fmt::Debug for LoginReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginReceipt")
            .field("hub", &self.hub)
            .finish_non_exhaustive()
    }
}
impl PartialEq for LoginReceipt {
    fn eq(&self, other: &Self) -> bool {
        self.hub == other.hub && Arc::ptr_eq(&self.owner, &other.owner)
    }
}
