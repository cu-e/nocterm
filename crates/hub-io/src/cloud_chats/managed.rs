//! The managed side of chats: which runners are agent seats. Waking a seat for a chat is the
//! device core's (its enrolled seat's pool, `StartInstance` before a command, ADR 0133).

use std::collections::HashMap;
use std::sync::Arc;

use nocterm_cloud_agents_model::AgentId;
use nocterm_hub_client::agents::AgentError;
use nocterm_hub_proto::v1;

use super::error::{self, CloudChatError};
use crate::devices::Captured;

/// One agent seat of the account: the runner it hosts and what a chat on it prompts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Seat {
    pub(super) runner: [u8; 16],
    pub(super) agent: AgentId,
    pub(super) name: String,
    pub(super) model: String,
    pub(super) models: Vec<String>,
}

fn runner_id(instance: &v1::Instance) -> Option<[u8; 16]> {
    instance.runner_id.as_slice().try_into().ok()
}

/// Whether a registered runner is one of the person's own: not the runner of a managed seat,
/// listed or gone.
pub(super) fn is_own_runner(
    label: &str,
    runner: &[u8; 16],
    seats: &HashMap<[u8; 16], Seat>,
) -> bool {
    !seats.contains_key(runner) && !crate::agents::is_seat_label(label)
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
                    runner: runner_id(instance)?,
                    agent: agent.id.clone(),
                    name: agent.name.clone(),
                    model: agent.primary_model()?.to_owned(),
                    models: agent.models.clone(),
                },
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runner_of_a_gone_seat_is_not_the_persons_own() {
        let seats = HashMap::new();
        assert!(!is_own_runner("Cloud agent 123", &[1; 16], &seats));
        assert!(is_own_runner("laptop", &[1; 16], &seats));
    }
}
