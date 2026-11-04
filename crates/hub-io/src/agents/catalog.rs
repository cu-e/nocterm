//! Listing, watching and pools.

use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use nocterm_cloud_agents_model::{AgentHints, CloudAgent, CloudAgentError, PoolChoice, PoolId};
use tokio::runtime::Handle;
use tokio::sync::watch;

use super::convert;
use super::error::agent_error;
use crate::devices::Captured;

pub(crate) async fn list(captured: Arc<Captured>) -> Result<Vec<CloudAgent>, CloudAgentError> {
    let address = captured.hub.address().clone();
    let client = captured.client.agents();
    let listings = client
        .list_agents_with_org_names(None)
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
        .filter_map(|listed| convert::agent(listed.listing, listed.org_name, &names))
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

/// Lossy revisions occupy one cell; completion retains the exact terminal error.
struct Hints {
    receiver: watch::Receiver<u64>,
    completion: crate::task::Spawned<Result<(), CloudAgentError>>,
    ended: bool,
}
impl Hints {
    fn spawn(
        handle: &Handle,
        hints: impl futures::Stream<Item = Result<u64, CloudAgentError>> + Send + 'static,
    ) -> Self {
        let (sender, receiver) = watch::channel(0);
        let completion = crate::task::spawn(handle, async move {
            let mut hints = std::pin::pin!(hints);
            loop {
                let item = tokio::select! {
                    item = hints.next() => item,
                    () = sender.closed() => return Ok(()),
                };
                let Some(revision) = item else {
                    return Ok(());
                };
                if sender.send(revision?).is_err() {
                    return Ok(());
                }
            }
        });
        Self {
            receiver,
            completion,
            ended: false,
        }
    }
}
impl AgentHints for Hints {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<u64>, CloudAgentError>> {
        async move {
            if self.ended {
                return Ok(None);
            }
            if self.receiver.changed().await.is_ok() {
                return Ok(Some(*self.receiver.borrow_and_update()));
            }
            let completion = (&mut self.completion).await;
            self.ended = true;
            match completion {
                Some(result) => result.map(|()| None),
                None => Ok(None),
            }
        }
        .boxed()
    }
}

pub(super) async fn watch(
    captured: Arc<Captured>,
    handle: Handle,
) -> Result<Box<dyn AgentHints>, CloudAgentError> {
    let address = captured.hub.address().clone();
    let stream = captured
        .client
        .agents()
        .watch_agents()
        .await
        .map_err(|error| agent_error(&address, error))?;
    let hints = futures::stream::try_unfold(stream, move |mut stream| {
        let address = address.clone();
        async move {
            stream
                .next()
                .await
                .map(|revision| revision.map(|revision| (revision, stream)))
                .map_err(|error| agent_error(&address, error))
        }
    });
    Ok(Box::new(Hints::spawn(&handle, hints)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_catalog_readers_receive_only_the_latest_revision_then_the_terminal_error() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (finished, burst_finished) = futures::channel::oneshot::channel();
        let hints =
            futures::stream::iter((1..=100).map(Ok)).chain(futures::stream::once(async move {
                let _ = finished.send(());
                Err(CloudAgentError::Unsupported)
            }));
        let mut hints = Hints::spawn(runtime.handle(), hints);
        runtime.block_on(async {
            burst_finished.await.unwrap();
            assert_eq!(hints.next().await.unwrap(), Some(100));
            assert!(matches!(
                hints.next().await,
                Err(CloudAgentError::Unsupported)
            ));
            assert_eq!(hints.next().await.unwrap(), None);
        });
    }

    #[tokio::test]
    async fn ending_the_hint_feed_delivers_the_latest_revision_before_eof() {
        let mut hints = Hints::spawn(&Handle::current(), futures::stream::iter([Ok(7)]));
        tokio::task::yield_now().await;
        assert_eq!(hints.next().await.unwrap(), Some(7));
        assert_eq!(hints.next().await.unwrap(), None);
    }

    #[tokio::test]
    async fn dropping_the_hint_reader_closes_its_original_feed() {
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        let hints = Hints::spawn(&Handle::current(), receiver);
        drop(hints);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !sender.is_closed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(sender.unbounded_send(Ok(7)).is_err());
    }
}
