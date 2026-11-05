//! The personal agents the person explicitly leases to an existing own-runner chat.
//! A verified transcript's model or provider label never chooses a credential.
use super::{Shared, error, leases};
use crate::cloud_device::{
    DeviceAccount, DeviceConversation, ProviderDirectory, ProviderModels, lease_models,
};
use nocterm_cloud_agents_model::{AgentId, AgentScope, CloudAgent, CloudChatError, Placement};
use nocterm_cloud_conversation_model::{
    CloudConversationError, ConversationFuture, ProviderOption,
};
use nocterm_cloud_device::{context::ChatRef, open::ModelChoice};
use nocterm_core::error::SharedCause;
use nocterm_hubs_model::HubId;
use std::sync::{Arc, Weak};

pub(super) struct HubProviders {
    shared: Weak<Shared>,
    account: Arc<DeviceAccount>,
    hub: HubId,
    chat: ChatRef,
}
impl HubProviders {
    pub(super) fn new(
        shared: &Arc<Shared>,
        account: Arc<DeviceAccount>,
        hub: &HubId,
        chat: ChatRef,
    ) -> Self {
        Self {
            shared: Arc::downgrade(shared),
            account,
            hub: hub.clone(),
            chat,
        }
    }
}
impl ProviderDirectory for HubProviders {
    fn options(&self) -> ConversationFuture<Vec<ProviderOption>> {
        let Some(shared) = self.shared.upgrade() else {
            return Box::pin(async { Err(CloudConversationError::Ended) });
        };
        let account = self.account.clone();
        let work = shared.call(&self.hub, move |_, captured, _| async move {
            account.check().map_err(error::hub)?;
            let agents = crate::agents::list_agents(captured).await?;
            account.check().map_err(error::hub)?;
            Ok(agents
                .into_iter()
                .filter(available)
                .map(|agent| ProviderOption {
                    id: agent.id.to_string(),
                    label: agent.name,
                })
                .collect())
        });
        Box::pin(async move { work.await.map_err(conversation) })
    }
    fn select(
        &self,
        id: String,
        revision: u64,
        held: Weak<DeviceConversation>,
    ) -> ConversationFuture<ProviderModels> {
        let Some(shared) = self.shared.upgrade() else {
            return Box::pin(async { Err(CloudConversationError::Ended) });
        };
        let (original, hub, chat) = (self.account.clone(), self.hub.clone(), self.chat);
        let work = shared.call(&self.hub, move |shared, captured, _| async move {
            original.check().map_err(error::hub)?;
            let agent = AgentId::new(id).map_err(|_| CloudChatError::Unsupported)?;
            let agents = crate::agents::list_agents(captured).await?;
            if !agents
                .iter()
                .any(|candidate| candidate.id == agent && available(candidate))
            {
                return Err(CloudChatError::Unsupported);
            }
            original.check().map_err(error::hub)?;
            let request = leases::LeaseRequest {
                hub: &hub,
                agent: &agent,
                chat,
                control_revision: revision,
            };
            let lease = leases::lease(&shared, original.clone(), request, held).await?;
            original.check().map_err(error::hub)?;
            Ok(ProviderModels {
                default: ModelChoice {
                    provider: lease.provider.clone(),
                    model: lease.model.clone(),
                },
                models: lease_models(&lease),
            })
        });
        Box::pin(async move { work.await.map_err(conversation) })
    }
}
fn available(agent: &CloudAgent) -> bool {
    agent.scope == AgentScope::Personal
        && agent.placement() == Placement::OwnRunner
        && agent.status.can_start()
}
fn conversation(error: CloudChatError) -> CloudConversationError {
    match error {
        CloudChatError::Stale => CloudConversationError::Stale,
        CloudChatError::Unsupported => CloudConversationError::Unsupported,
        CloudChatError::Capacity => CloudConversationError::Capacity,
        CloudChatError::Conversation(error) => error,
        other => CloudConversationError::Refused(SharedCause::new(other)),
    }
}
