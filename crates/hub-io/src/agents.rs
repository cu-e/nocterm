//! The agents, providers, agent key and agent servers of the account, over `AgentService` and
//! `ServerService`, as the ports of `nocterm-hub-agents-model`.
//!
//! Every call captures the account's original membership before it returns its future and
//! checks it again after every await, like the devices of the account: a result for an
//! account that signed out or was replaced meanwhile is `HubError::Stale`.

mod convert;
mod servers;

use std::sync::Arc;

use nocterm_hub_agents_model::{
    AgentDraft, AgentId, AgentsFuture, HubAgent, HubAgentDirectory, HubAgentsError, Provider,
    ProviderDraft, ProviderId, ProviderTest,
};
use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubId;
use tokio::runtime::Handle;

use crate::classify::classify;
use crate::devices::{Captured, run};
use crate::links::HubLinks;
use crate::pool::Pool;

/// Implements the agents ports over the clients of the hubs this device is signed in to.
pub struct HubAgentsLink {
    handle: Handle,
    pool: Arc<Pool>,
    hints: Arc<crate::chats::ChatHints>,
}

impl HubAgentsLink {
    pub fn new(links: &HubLinks) -> Self {
        let (handle, pool) = links.agent_parts();
        Self {
            handle,
            pool,
            hints: links.chat_hints(),
        }
    }

    /// Runs `call` with a fresh client of the account on `hub`.
    fn call<T, F, Fut>(&self, hub: &HubId, call: F) -> AgentsFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(HubClient) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T, CallError>> + Send + 'static,
    {
        let captured = Captured::new(self.pool.clone(), hub);
        let outcome = run(captured, &self.handle, move |captured| async move {
            let client = captured.client.clone();
            let address = captured.hub.address().clone();
            let fresh = captured.observed.during(client.ensure_fresh()).await?;
            if let Err(error) = fresh {
                return Ok(Err(HubAgentsError::Hub(classify(&address, error))));
            }
            let result = captured.observed.during(call(client)).await?;
            Ok(result.map_err(|error| error.into_agents(&address)))
        });
        Box::pin(async move { outcome.await.map_err(HubAgentsError::Hub)? })
    }
}

/// A failed call before it is told in the port's terms.
pub(crate) enum CallError {
    Client(ClientError),
    Agents(HubAgentsError),
}

impl From<tonic::Status> for CallError {
    fn from(status: tonic::Status) -> Self {
        Self::Client(ClientError::from(status))
    }
}

impl From<HubAgentsError> for CallError {
    fn from(error: HubAgentsError) -> Self {
        Self::Agents(error)
    }
}

impl CallError {
    fn into_agents(self, address: &nocterm_hubs_model::HubAddress) -> HubAgentsError {
        match self {
            Self::Agents(error) => error,
            Self::Client(ClientError::Status {
                hub_code: Some(v2::ErrorCode::LimitReached),
                ..
            }) => HubAgentsError::LimitReached,
            Self::Client(error) => HubAgentsError::Hub(classify(address, error)),
        }
    }
}

impl HubAgentDirectory for HubAgentsLink {
    fn agents(&self, hub: &HubId) -> AgentsFuture<Vec<HubAgent>> {
        self.call(hub, |client| async move {
            let listed = client
                .agents()
                .list_agents(v2::Empty {})
                .await?
                .into_inner();
            listed
                .agents
                .into_iter()
                .map(convert::agent)
                .collect::<Result<_, _>>()
                .map_err(CallError::from)
        })
    }

    fn create_agent(&self, hub: &HubId, draft: AgentDraft) -> AgentsFuture<HubAgent> {
        self.call(hub, |client| async move {
            let input = convert::agent_input(draft.validate().map_err(HubAgentsError::from)?);
            let made = client.agents().create_agent(input).await?.into_inner();
            Ok(convert::agent(made)?)
        })
    }

    fn update_agent(
        &self,
        hub: &HubId,
        agent: &AgentId,
        draft: AgentDraft,
    ) -> AgentsFuture<HubAgent> {
        let id = agent.to_string();
        self.call(hub, |client| async move {
            let input = convert::agent_input(draft.validate().map_err(HubAgentsError::from)?);
            let request = v2::UpdateAgentRequest {
                id,
                agent: Some(input),
            };
            let saved = client.agents().update_agent(request).await?.into_inner();
            Ok(convert::agent(saved)?)
        })
    }

    fn delete_agent(&self, hub: &HubId, agent: &AgentId) -> AgentsFuture<()> {
        let id = agent.to_string();
        self.call(hub, |client| async move {
            client.agents().delete_agent(v2::IdRequest { id }).await?;
            Ok(())
        })
    }

    fn providers(&self, hub: &HubId) -> AgentsFuture<Vec<Provider>> {
        self.call(hub, |client| async move {
            let listed = client
                .agents()
                .list_providers(v2::Empty {})
                .await?
                .into_inner();
            listed
                .providers
                .into_iter()
                .map(convert::provider)
                .collect::<Result<_, _>>()
                .map_err(CallError::from)
        })
    }

    fn create_provider(&self, hub: &HubId, draft: ProviderDraft) -> AgentsFuture<Provider> {
        self.call(hub, |client| async move {
            let input = convert::provider_input(draft.validate().map_err(HubAgentsError::from)?);
            let made = client.agents().create_provider(input).await?.into_inner();
            Ok(convert::provider(made)?)
        })
    }

    fn update_provider(
        &self,
        hub: &HubId,
        provider: &ProviderId,
        draft: ProviderDraft,
    ) -> AgentsFuture<Provider> {
        let id = provider.to_string();
        self.call(hub, |client| async move {
            let input = convert::provider_input(draft.validate().map_err(HubAgentsError::from)?);
            let request = v2::UpdateProviderRequest {
                id,
                provider: Some(input),
            };
            let saved = client.agents().update_provider(request).await?.into_inner();
            Ok(convert::provider(saved)?)
        })
    }

    fn delete_provider(&self, hub: &HubId, provider: &ProviderId) -> AgentsFuture<()> {
        let id = provider.to_string();
        self.call(hub, |client| async move {
            client
                .agents()
                .delete_provider(v2::IdRequest { id })
                .await?;
            Ok(())
        })
    }

    fn test_provider(&self, hub: &HubId, provider: &ProviderId) -> AgentsFuture<ProviderTest> {
        let id = provider.to_string();
        self.call(hub, |client| async move {
            let request = v2::TestProviderRequest {
                target: Some(v2::test_provider_request::Target::ProviderId(id)),
            };
            let tested = client.agents().test_provider(request).await?.into_inner();
            Ok(ProviderTest {
                ok: tested.ok,
                message: tested.message,
                models: tested.models,
            })
        })
    }
}

#[cfg(test)]
mod tests;
