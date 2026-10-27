//! The hub as the first step of an SSH route: [`RelayDialer`] over the hub's relay stream.
//!
//! A dial finds the hub by its address among the hubs this device is signed in to, asks it to
//! open a connection to the destination and hands the transport the two bounded frame channels
//! of a [`RelayStream`]. A task on the adapter's runtime then carries frames between those
//! channels and the hub's stream ([`pump`]); the two runtimes share nothing else.
//!
//! The hub learns the destination address and what it is told to forward, never what is inside
//! the SSH connection: host keys and sign-in are the transport's, checked against the
//! destination.

mod failure;
mod pump;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use nocterm_hub_client::{HubClient, RelayTarget};
use nocterm_hubs_model::{HubAddress, HubId};
use nocterm_session::{RelayDialer, RelayError, RelayStream, Target};
use tokio::runtime::Handle;
use tokio::task::AbortHandle;

use crate::links::HubLinks;
use crate::pool::Pool;

/// Opens relayed streams through the hubs this device is signed in to.
pub struct HubRelay {
    handle: Handle,
    pool: Arc<Pool>,
}

impl HubRelay {
    /// A relay that runs on the threads of `links` and uses the hubs `links` signed in to.
    ///
    /// Once the links are dropped, dials fail with [`RelayError::Closed`].
    pub fn new(links: &HubLinks) -> Self {
        let (handle, pool) = links.relay_parts();
        Self { handle, pool }
    }
}

/// Everything one dial needs, owned by its task.
struct Opening {
    pool: Arc<Pool>,
    handle: Handle,
    hub: String,
    target: RelayTarget,
}

impl RelayDialer for HubRelay {
    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
        let opening = Opening {
            pool: Arc::clone(&self.pool),
            handle: self.handle.clone(),
            hub: hub.to_owned(),
            target: RelayTarget {
                host: target.host.clone(),
                port: target.port,
                node: node.map(str::to_owned),
            },
        };
        let (sender, result) = oneshot::channel();
        let task = self
            .handle
            .spawn(async move {
                let _ = sender.send(opening.open().await);
            })
            .abort_handle();
        Dial {
            result,
            task: Some(task),
        }
        .boxed()
    }
}

impl Opening {
    /// Opens the hub's stream and starts carrying frames between it and the returned stream.
    async fn open(self) -> Result<RelayStream, RelayError> {
        let (hub, client) = self.client().await?;
        let conn = match client.relay(&self.target).await {
            Ok(conn) => conn,
            Err(error) => {
                return Err(failure::opening(
                    &self.pool,
                    &hub,
                    &self.hub,
                    self.target.node.as_deref(),
                    error,
                ));
            }
        };
        tracing::debug!(hub = %hub, peer = %conn.peer_ip(), "relay opened");
        let (stream, peer) = RelayStream::pair();
        let (writer, reader) = conn.split();
        self.handle.spawn(pump::carry(peer, writer, reader));
        Ok(stream)
    }

    /// The client of the hub named by `self.hub`, if this device is signed in to it.
    async fn client(&self) -> Result<(HubId, HubClient), RelayError> {
        let not_signed_in = || RelayError::NotSignedIn {
            hub: self.hub.clone(),
        };
        let address = HubAddress::parse(&self.hub).map_err(|_| not_signed_in())?;
        let hub = HubId::from(address);
        match self.pool.fresh(&hub).await {
            Ok(client) => Ok((hub, client)),
            Err(error) => Err(failure::without_client(&self.hub, error)),
        }
    }
}

/// The answer of a dial running on the adapter's runtime; abandons the dial when dropped.
///
/// A stream already handed out is not part of the dial: its pump runs on its own.
struct Dial {
    result: oneshot::Receiver<Result<RelayStream, RelayError>>,
    task: Option<AbortHandle>,
}

impl Future for Dial {
    type Output = Result<RelayStream, RelayError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // A task that vanished without answering was stopped with the runtime.
        self.result
            .poll_unpin(cx)
            .map(|received| received.unwrap_or(Err(RelayError::Closed)))
    }
}

impl Drop for Dial {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests;
