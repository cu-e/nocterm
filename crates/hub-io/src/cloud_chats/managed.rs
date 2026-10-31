//! The managed side of chats: which runners are agent seats, and starting a seat for a chat.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use nocterm_cloud_agents_model::{AgentId, CloudAgentError};
use nocterm_hub_client::agents::AgentError;
use nocterm_hub_proto::v1;

use super::error::{self, CloudChatError};
use crate::devices::Captured;

/// One agent seat of the account: the runner it hosts and what a chat on it prompts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Seat {
    pub(super) agent: AgentId,
    pub(super) name: String,
    pub(super) model: String,
    pub(super) pool: String,
}

fn parse<T: FromStr>(text: &str) -> Result<T, CloudChatError> {
    text.parse()
        .map_err(|_| CloudChatError::Agent(CloudAgentError::ProvisioningFailed))
}

fn runner_id(instance: &v1::Instance) -> Option<[u8; 16]> {
    instance.runner_id.as_slice().try_into().ok()
}

/// Starts, or wakes, the seat of `agent` for `chat` and returns the runner that hosts it.
///
/// The hub makes this idempotent per chat: a running seat only receives access for the chat.
pub(super) async fn start_for_chat(
    captured: &Arc<Captured>,
    agent: &AgentId,
    pool: Option<&str>,
    chat: [u8; 16],
) -> Result<[u8; 16], CloudChatError> {
    let address = captured.hub.address().clone();
    let pool = pool.map(parse::<nocterm_hub_model::PoolId>).transpose()?;
    let instance = captured
        .client
        .agents()
        .start_instance_for_chat(parse(agent.as_str())?, pool, chat)
        .await
        .map_err(|failure| match failure {
            AgentError::Unsupported => CloudChatError::Unsupported,
            other => error::agent(&address, other),
        })?;
    runner_id(&instance).ok_or(CloudChatError::NotEnrolled)
}

/// The seats of the account by runner. A hub without agents has none.
pub(super) async fn seats(
    captured: &Arc<Captured>,
) -> Result<HashMap<[u8; 16], Seat>, CloudChatError> {
    let address = captured.hub.address().clone();
    let listed = match captured.client.agents().list_instances(None).await {
        Ok(listed) => listed,
        Err(AgentError::Unsupported) => return Ok(HashMap::new()),
        Err(other) => return Err(error::agent(&address, other)),
    };
    let agents = crate::agents::list_agents(captured.clone())
        .await
        .map_err(CloudChatError::Agent)?;
    Ok(listed
        .iter()
        .filter_map(|instance| {
            let agent = agents
                .iter()
                .find(|agent| agent.id.as_str() == instance.agent_id)?;
            Some((
                runner_id(instance)?,
                Seat {
                    agent: agent.id.clone(),
                    name: agent.name.clone(),
                    model: agent.primary_model()?.to_owned(),
                    pool: instance.pool_id.clone(),
                },
            ))
        })
        .collect())
}
