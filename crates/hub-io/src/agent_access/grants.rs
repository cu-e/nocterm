//! The chat accesses agents hold, listed and revoked through the hub's agent service.

use std::sync::Arc;

use nocterm_hubs_model::{AgentAccessError, AgentAccessGrant, HubId};
use tokio::runtime::Handle;

use crate::classify::remote;
use crate::devices::{Captured, run};
use crate::pool::Pool;
use nocterm_hubs_model::HubError;

fn failed(error: nocterm_hub_client::agents::AgentError) -> HubError {
    remote(error)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) async fn list(
    pool: Arc<Pool>,
    handle: Handle,
    hub: HubId,
) -> Result<Vec<AgentAccessGrant>, AgentAccessError> {
    run(Captured::new(pool, &hub), &handle, |captured| async move {
        let listed = captured
            .observed
            .during(captured.client.agents().list_access_grants())
            .await?
            .map_err(failed)?;
        Ok(listed
            .into_iter()
            .map(|grant| AgentAccessGrant {
                grant_id: grant.grant_id,
                agent_id: grant.agent_id,
                chat_id: hex(&grant.chat_id),
                targets: grant.targets,
                issued_at_ms: grant.issued_at_ms,
                expires_at_ms: grant.expires_at_ms,
            })
            .collect())
    })
    .await
    .map_err(AgentAccessError::Hub)
}

pub(super) async fn revoke(
    pool: Arc<Pool>,
    handle: Handle,
    hub: HubId,
    grant_id: String,
) -> Result<(), AgentAccessError> {
    run(Captured::new(pool, &hub), &handle, |captured| async move {
        captured
            .observed
            .during(captured.client.agents().revoke_access_grant(&grant_id))
            .await?
            .map_err(failed)
    })
    .await
    .map_err(AgentAccessError::Hub)
}
