//! Listing, watching and pools.

use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::future::BoxFuture;
use nocterm_cloud_agents_model::{AgentHints, CloudAgent, CloudAgentError, PoolChoice, PoolId};
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

use super::convert;
use super::error::agent_error;
use crate::devices::Captured;

pub(super) async fn list(captured: Arc<Captured>) -> Result<Vec<CloudAgent>, CloudAgentError> {
    let address = captured.hub.address().clone();
    let client = captured.client.agents();
    let listings = client
        .list_agents(None)
        .await
        .map_err(|error| agent_error(&address, error))?;
    // Names are only decoration: a hub that cannot list pools still lists its agents.
    let names: HashMap<String, String> = client
        .list_pools(None)
        .await
        .map(|pools| {
            pools
                .into_iter()
                .map(|pool| (pool.pool_id, pool.name))
                .collect()
        })
        .unwrap_or_default();
    Ok(listings
        .into_iter()
        .filter_map(|listing| convert::agent(listing, &names))
        .collect())
}

pub(super) async fn pools(captured: Arc<Captured>) -> Result<Vec<PoolChoice>, CloudAgentError> {
    let address = captured.hub.address().clone();
    let pools = captured
        .client
        .agents()
        .list_pools(None)
        .await
        .map_err(|error| agent_error(&address, error))?;
    Ok(pools
        .into_iter()
        .filter_map(|pool| {
            Some(PoolChoice {
                pool: PoolId::new(pool.pool_id).ok()?,
                name: pool.name,
                accepts_personal: true,
            })
        })
        .collect())
}

/// Hints forwarded from a task on the adapter's runtime, so any executor may await them.
struct Hints {
    receiver: mpsc::UnboundedReceiver<Result<Option<u64>, CloudAgentError>>,
    task: AbortHandle,
}

impl Drop for Hints {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl AgentHints for Hints {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<u64>, CloudAgentError>> {
        async move { self.receiver.next().await.unwrap_or(Ok(None)) }.boxed()
    }
}

pub(super) async fn watch(
    captured: Arc<Captured>,
    handle: Handle,
) -> Result<Box<dyn AgentHints>, CloudAgentError> {
    let address = captured.hub.address().clone();
    let mut stream = captured
        .client
        .agents()
        .watch_agents()
        .await
        .map_err(|error| agent_error(&address, error))?;
    let (sender, receiver) = mpsc::unbounded();
    let task = handle
        .spawn(async move {
            loop {
                let next = stream
                    .next()
                    .await
                    .map_err(|error| agent_error(&address, error));
                let done = !matches!(next, Ok(Some(_)));
                if sender.unbounded_send(next).is_err() || done {
                    return;
                }
            }
        })
        .abort_handle();
    Ok(Box::new(Hints { receiver, task }))
}
