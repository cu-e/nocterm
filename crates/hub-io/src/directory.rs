//! What a signed-in device reads and does on a hub.

use std::sync::Arc;

use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hub_proto::v1;
use nocterm_hubs_model::{
    Account, HubError, HubId, HubSessionId, HubSessionSummary, NodeSummary, OrgId,
};

use crate::convert;
use crate::pool::Pool;

pub(crate) struct Directory {
    pool: Arc<Pool>,
}

impl Directory {
    pub(crate) fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }

    pub(crate) async fn account(&self, hub: &HubId) -> Result<Account, HubError> {
        let client = self.pool.fresh(hub).await?;
        who_am_i(&client, hub, &self.pool).await
    }

    pub(crate) async fn nodes(
        &self,
        hub: &HubId,
        org: &OrgId,
    ) -> Result<Vec<NodeSummary>, HubError> {
        let client = self.pool.fresh(hub).await?;
        let request = v1::ListNodesRequest {
            org_id: org.as_str().to_owned(),
        };
        let list = client
            .fleet()
            .list_nodes(request)
            .await
            .map_err(|status| self.pool.failed(hub, ClientError::from(status)))?
            .into_inner();
        Ok(convert::nodes(list))
    }

    pub(crate) async fn sessions(
        &self,
        hub: &HubId,
        org: &OrgId,
    ) -> Result<Vec<HubSessionSummary>, HubError> {
        let client = self.pool.fresh(hub).await?;
        let request = v1::ListSessionsRequest {
            org_id: org.as_str().to_owned(),
            include_ended: false,
        };
        let list = client
            .sessions()
            .list_sessions(request)
            .await
            .map_err(|status| self.pool.failed(hub, ClientError::from(status)))?
            .into_inner();
        Ok(convert::sessions(list))
    }

    pub(crate) async fn end(&self, hub: &HubId, session: &HubSessionId) -> Result<(), HubError> {
        let client = self.pool.fresh(hub).await?;
        let request = v1::SessionRef {
            session_id: session.as_str().to_owned(),
        };
        client
            .sessions()
            .close_session(request)
            .await
            .map_err(|status| self.pool.failed(hub, ClientError::from(status)))?;
        Ok(())
    }
}

/// Who the device is signed in as on a hub.
pub(crate) async fn who_am_i(
    client: &HubClient,
    hub: &HubId,
    pool: &Pool,
) -> Result<Account, HubError> {
    let me = client
        .directory()
        .who_am_i(v1::Empty {})
        .await
        .map_err(|status| pool.failed(hub, ClientError::from(status)))?
        .into_inner();
    Ok(convert::account(me))
}
