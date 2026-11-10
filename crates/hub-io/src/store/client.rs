//! Client selection is separate from the one shared store RPC implementation.
use std::sync::Arc;

use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hubs_model::{HubError, source_identity::LocalAccountSource};

use super::OpError;
use crate::{
    account::source::{CapturedAccount, CapturedMembership},
    pool::{Pool, captured::CapturedClient},
};

pub(super) struct StoreClient {
    fixed: CapturedClient,
}
impl StoreClient {
    pub(super) fn captured(
        pool: Arc<Pool>,
        native: &CapturedAccount,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Self, HubError> {
        let fixed = CapturedClient::account(pool.clone(), native, source)?;
        Ok(Self { fixed })
    }
    pub(super) async fn fresh(&self) -> Result<HubClient, OpError> {
        self.fixed.fresh().await.map_err(OpError::from)
    }

    pub(super) fn membership(
        pool: Arc<Pool>,
        native: &CapturedMembership,
    ) -> Result<Self, HubError> {
        native.check()?;
        let fixed = CapturedClient::current(pool.clone(), native.hub().clone())?;
        if fixed.hub() != native.hub() {
            return Err(HubError::Stale);
        }
        native.check()?;
        Ok(Self { fixed })
    }

    pub(super) fn failure(&self, error: ClientError) -> OpError {
        if error.is_conflict() {
            return OpError::Conflict;
        }
        OpError::Hub(self.fixed.failure(error))
    }
}
