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

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use nocterm_hub_client::RelayTarget;
use nocterm_hubs_model::{HubAddress, HubId};
use nocterm_session::{RelayDialer, RelayError, RelayStream, Target};
use tokio::runtime::Handle;

use crate::account::source::LocalSourceProvider;
use crate::links::HubLinks;
use crate::pool::{Pool, captured::CapturedClient};
use nocterm_hubs_model::{HubError, source_identity::LocalAccountSource};

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

/// Capture capability for an account-bound terminal, composed by the application.
pub(crate) struct RelayCapture {
    handle: Handle,
    pool: Arc<Pool>,
}
impl RelayCapture {
    pub(crate) fn new(handle: Handle, pool: Arc<Pool>) -> Self {
        Self { handle, pool }
    }
    pub(crate) fn account(
        &self,
        provider: &LocalSourceProvider,
        source: Arc<dyn LocalAccountSource>,
    ) -> Result<Arc<dyn RelayDialer>, HubError> {
        let native = provider.capture_native()?;
        let client = CapturedClient::account(self.pool.clone(), &native, source)?;
        Ok(Arc::new(AccountRelay {
            handle: self.handle.clone(),
            client: Arc::new(client),
        }))
    }
}
struct AccountRelay {
    handle: Handle,
    client: Arc<CapturedClient>,
}
struct CapturedRelay {
    handle: Handle,
    client: Arc<CapturedClient>,
    hub: String,
    target: RelayTarget,
    principal: String,
}
impl HubRelay {
    fn captured(
        &self,
        name: &str,
        node: Option<&str>,
        target: &Target,
    ) -> Result<Arc<dyn RelayDialer>, RelayError> {
        if self.pool.is_stopped() {
            return Err(RelayError::Closed);
        }
        let missing = || RelayError::NotSignedIn {
            hub: name.to_owned(),
        };
        let hub = HubId::from(HubAddress::parse(name).map_err(|_| missing())?);
        let client = CapturedClient::current(self.pool.clone(), hub)
            .map_err(|error| failure::without_client(name, error))?;
        Ok(Arc::new(CapturedRelay::new(
            self.handle.clone(),
            Arc::new(client),
            name,
            node,
            target,
        )))
    }
}
impl RelayDialer for HubRelay {
    fn capture(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> Result<Arc<dyn RelayDialer>, RelayError> {
        self.captured(hub, node, target)
    }
    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
        match self.capture(hub, node, target) {
            Ok(captured) => captured.dial(hub, node, target),
            Err(error) => async move { Err(error) }.boxed(),
        }
    }
}
impl RelayDialer for AccountRelay {
    fn capture(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> Result<Arc<dyn RelayDialer>, RelayError> {
        let valid =
            HubAddress::parse(hub).ok().map(HubId::from).as_ref() == Some(self.client.hub());
        if !valid {
            return Err(RelayError::NotSignedIn {
                hub: hub.to_owned(),
            });
        }
        self.client
            .check()
            .map_err(|error| failure::without_client(hub, error))?;
        Ok(Arc::new(CapturedRelay::new(
            self.handle.clone(),
            self.client.clone(),
            hub,
            node,
            target,
        )))
    }
    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
        match self.capture(hub, node, target) {
            Ok(captured) => captured.dial(hub, node, target),
            Err(error) => async move { Err(error) }.boxed(),
        }
    }
}
impl CapturedRelay {
    fn new(
        handle: Handle,
        client: Arc<CapturedClient>,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> Self {
        Self {
            handle,
            client,
            hub: hub.to_owned(),
            principal: target.user.clone(),
            target: RelayTarget {
                host: target.host.clone(),
                port: target.port,
                node: node.map(str::to_owned),
            },
        }
    }
    fn matches(&self, hub: &str, node: Option<&str>, target: &Target) -> bool {
        self.hub == hub
            && self.target.node.as_deref() == node
            && self.target.host == target.host
            && self.target.port == target.port
            && self.principal == target.user
    }
    async fn open(self) -> Result<RelayStream, RelayError> {
        let client = self
            .client
            .fresh()
            .await
            .map_err(|error| failure::without_client(&self.hub, error))?;
        // The SDK and membership are already captured; this never selects B.
        self.client
            .check()
            .map_err(|error| failure::without_client(&self.hub, error))?;
        let conn = self
            .client
            .during(client.relay(&self.target))
            .await
            .map_err(|error| failure::without_client(&self.hub, error))?
            .map_err(|error| {
                failure::opening(&self.client, &self.hub, self.target.node.as_deref(), error)
            })?;
        self.client
            .check()
            .map_err(|error| failure::without_client(&self.hub, error))?;
        let (stream, peer) = RelayStream::pair();
        let (writer, reader) = conn.split();
        let client = self.client.clone();
        self.handle.spawn(async move {
            let _ = client.during(pump::carry(peer, writer, reader)).await;
        });
        Ok(stream)
    }
}
impl RelayDialer for CapturedRelay {
    fn capture(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> Result<Arc<dyn RelayDialer>, RelayError> {
        if !self.matches(hub, node, target) {
            return Err(RelayError::CaptureUnsupported);
        }
        self.client
            .check()
            .map_err(|error| failure::without_client(hub, error))?;
        Ok(Arc::new(Self::new(
            self.handle.clone(),
            self.client.clone(),
            hub,
            node,
            target,
        )))
    }
    fn dial(
        &self,
        hub: &str,
        node: Option<&str>,
        target: &Target,
    ) -> BoxFuture<'static, Result<RelayStream, RelayError>> {
        let captured = match self.capture(hub, node, target) {
            Ok(value) => value,
            Err(error) => return async move { Err(error) }.boxed(),
        };
        // Recreate only an immutable exact-route view; no account lookup occurs.
        drop(captured);
        let opening = Self::new(self.handle.clone(), self.client.clone(), hub, node, target);
        // Dropping the dial aborts only opening; an issued stream owns its pump.
        crate::task::spawn(&self.handle, async move { opening.open().await })
            .map(|result| result.unwrap_or(Err(RelayError::Closed)))
            .boxed()
    }
}

#[cfg(test)]
mod tests;
