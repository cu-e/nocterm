//! The personal SSH CA for cloud agents and the servers that trust it.
//!
//! Owns: the `agent_ssh_trust` registry in the account's own encrypted collection (a hub sees
//! ciphertext only), the rules that move a server between trusted, pending removal, removed
//! and drifted, the per-server `authorized_keys` line each trusted server is given (and the
//! rewrite of a shared-principal line on its next connection), and the adapter of the
//! `AgentHostTrust` and `AgentGrants` ports. `authorized_keys` is reached only through
//! `AgentHosts`. The CA, its document, principals, lines and trust records are the
//! contract's (`nocterm_keys::agents`, contract ADR 0009, ADR 0147); contract types stay here
//! (ADR 0096).

mod grants;
mod registry;
#[cfg(test)]
mod tests;
mod trust;

use std::sync::Arc;

use futures::FutureExt as _;
use nocterm_hub_model::DeviceId;
use nocterm_hubs_model::{
    AccountDevices as _, AgentAccessError, AgentAccessFuture, AgentAccessGrant, AgentCa,
    AgentGrants, AgentHostTrust, AgentTrustOverview, HostChoice, HubError, HubFuture, HubId,
    ServerTrust,
};
use nocterm_ssh_keys::authorized_keys::AgentHosts;
use tokio::runtime::Handle;

use crate::account::operations::OpenedWork;
use crate::links::HubLinks;
use crate::pool::Pool;
use crate::task::spawn;
use trust::{Clock, Trust};

/// Where the trust service gets the signed-in account of a hub.
pub(crate) trait AccountWork: Send + Sync {
    fn work(&self, hub: &HubId) -> Result<OpenedWork, HubError>;
    /// This device's identifier on the hub.
    fn device(&self, hub: &HubId) -> HubFuture<DeviceId>;
}

struct LinkedAccounts(Arc<HubLinks>);

impl AccountWork for LinkedAccounts {
    fn work(&self, hub: &HubId) -> Result<OpenedWork, HubError> {
        self.0.open_account(hub)?.work()
    }

    fn device(&self, hub: &HubId) -> HubFuture<DeviceId> {
        let listed = self.0.list(hub);
        async move {
            listed
                .await?
                .into_iter()
                .find(|device| device.is_current)
                .and_then(|device| device.id.parse().ok())
                .ok_or(HubError::NotFound)
        }
        .boxed()
    }
}

/// Implements the agent access ports over the hub links and the user's open sessions.
pub struct AgentAccess {
    handle: Handle,
    pool: Arc<Pool>,
    trust: Arc<Trust>,
}

impl AgentAccess {
    /// The ports over `links`; `hosts` reaches the `authorized_keys` of connected servers.
    pub fn new(links: Arc<HubLinks>, hosts: Arc<dyn AgentHosts>) -> Self {
        let (handle, pool) = links.relay_parts();
        let clock: Clock = Arc::new(nocterm_hub_client::now_ms);
        Self {
            handle,
            pool,
            trust: Arc::new(Trust::new(Arc::new(LinkedAccounts(links)), hosts, clock)),
        }
    }

    #[cfg(test)]
    fn with_parts(
        source: Arc<dyn AccountWork>,
        hosts: Arc<dyn AgentHosts>,
        clock: Clock,
        pool: Arc<Pool>,
    ) -> Self {
        Self {
            handle: Handle::current(),
            pool,
            trust: Arc::new(Trust::new(source, hosts, clock)),
        }
    }

    fn run<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = Result<T, AgentAccessError>> + Send + 'static,
    ) -> AgentAccessFuture<T> {
        let task = spawn(&self.handle, work);
        Box::pin(async move {
            task.await
                .unwrap_or(Err(AgentAccessError::Hub(HubError::Stale)))
        })
    }
}

impl AgentAccess {
    /// The servers of `hub`'s registry that are trusted now, as a chat binds them: each named
    /// by `SHA-256(connection)[..16]`, with the port this device last reached it on (ADR 0147).
    pub(crate) fn server_choices(
        &self,
        hub: &HubId,
    ) -> AgentAccessFuture<Vec<nocterm_cloud_conversation_model::ServerChoice>> {
        let (trust, hub) = (self.trust.clone(), hub.clone());
        self.run(async move { trust.server_choices(&hub).await })
    }
}

impl AgentHostTrust for AgentAccess {
    fn connected_hosts(&self) -> Vec<HostChoice> {
        self.trust.sessions()
    }

    fn overview(&self, hub: &HubId) -> AgentAccessFuture<AgentTrustOverview> {
        let (trust, hub) = (self.trust.clone(), hub.clone());
        self.run(async move { trust.overview(&hub).await })
    }

    fn allow(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<()> {
        let (trust, hub, connection) = (self.trust.clone(), hub.clone(), connection.to_owned());
        self.run(async move { trust.allow(&hub, &connection).await })
    }

    fn deny(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<ServerTrust> {
        let (trust, hub, connection) = (self.trust.clone(), hub.clone(), connection.to_owned());
        self.run(async move { trust.deny(&hub, &connection).await })
    }

    fn reconcile(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<ServerTrust> {
        let (trust, hub, connection) = (self.trust.clone(), hub.clone(), connection.to_owned());
        self.run(async move { trust.reconcile(&hub, &connection).await })
    }

    fn reissue_ca(&self, hub: &HubId) -> AgentAccessFuture<AgentCa> {
        let (trust, hub) = (self.trust.clone(), hub.clone());
        self.run(async move { trust.reissue(&hub).await })
    }

    fn ca_line(&self, hub: &HubId, connection: &str) -> AgentAccessFuture<String> {
        let (trust, hub, connection) = (self.trust.clone(), hub.clone(), connection.to_owned());
        self.run(async move { trust.ca_line(&hub, &connection).await })
    }
}

impl AgentGrants for AgentAccess {
    fn list(&self, hub: &HubId) -> AgentAccessFuture<Vec<AgentAccessGrant>> {
        let (pool, handle, hub) = (self.pool.clone(), self.handle.clone(), hub.clone());
        Box::pin(grants::list(pool, handle, hub))
    }

    fn revoke(&self, hub: &HubId, grant_id: &str) -> AgentAccessFuture<()> {
        let (pool, handle, hub) = (self.pool.clone(), self.handle.clone(), hub.clone());
        Box::pin(grants::revoke(pool, handle, hub, grant_id.to_owned()))
    }
}
