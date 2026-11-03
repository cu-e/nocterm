//! The managed side of chats: which runners are agent seats, and starting a seat for a chat.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use nocterm_cloud_agents_model::{AgentId, CloudAgentError, ReplyProblem};
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
}

fn parse<T: FromStr>(text: &str, problem: ReplyProblem) -> Result<T, CloudChatError> {
    text.parse()
        .map_err(|_| CloudChatError::Agent(CloudAgentError::UnexpectedReply(problem)))
}

fn runner_id(instance: &v1::Instance) -> Option<[u8; 16]> {
    instance.runner_id.as_slice().try_into().ok()
}

/// The instance of the agent that hosts a chat: it has an enrolled runner (the one asked for,
/// when `runner` is given) and is not on its way out.
fn hosting(
    instances: &[v1::Instance],
    runner: Option<[u8; 16]>,
) -> Option<(&v1::Instance, [u8; 16])> {
    instances.iter().find_map(|instance| {
        let own = runner_id(instance)?;
        let alive = instance.state != v1::InstanceState::Deleting as i32;
        (alive && runner.is_none_or(|wanted| wanted == own)).then_some((instance, own))
    })
}

/// Wakes the seat of `agent` for `chat` and returns the runner that hosts it.
///
/// The hub makes waking idempotent per chat: a running seat only receives access for the chat.
/// A chat never provisions a seat: when the agent has no enrolled seat (it was removed, or its
/// start never finished) the call fails with [`CloudChatError::AgentNotStarted`], because the
/// hub would otherwise create an instance that nothing enrols and that holds the host's
/// capacity. Starting the agent is what creates and enrols a seat.
pub(super) async fn start_for_chat(
    captured: &Arc<Captured>,
    agent: &AgentId,
    runner: Option<[u8; 16]>,
    chat: [u8; 16],
) -> Result<[u8; 16], CloudChatError> {
    let address = captured.hub.address().clone();
    let wire_agent: nocterm_hub_model::AgentId = parse(agent.as_str(), ReplyProblem::AgentId)?;
    let agents = captured.client.agents();
    let listed =
        agents
            .list_instances(Some(wire_agent))
            .await
            .map_err(|failure| match failure {
                AgentError::Unsupported => CloudChatError::Unsupported,
                other => error::agent(&address, other),
            })?;
    let (seat, hosted_by) = hosting(&listed, runner).ok_or(CloudChatError::AgentNotStarted)?;
    let pool = parse::<nocterm_hub_model::PoolId>(&seat.pool_id, ReplyProblem::PoolId)?;
    let instance = agents
        .start_instance_for_chat(wire_agent, Some(pool), chat)
        .await
        .map_err(|failure| match failure {
            AgentError::Unsupported => CloudChatError::Unsupported,
            other => error::agent(&address, other),
        })?;
    Ok(runner_id(&instance).unwrap_or(hosted_by))
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
                },
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(state: v1::InstanceState, runner: &[u8]) -> v1::Instance {
        v1::Instance {
            state: state as i32,
            runner_id: runner.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn an_instance_awaiting_enrolment_hosts_no_chat() {
        let waiting = [instance(v1::InstanceState::AwaitingEnrolment, &[])];
        assert!(hosting(&waiting, None).is_none());
    }

    #[test]
    fn an_instance_being_deleted_hosts_no_chat() {
        let going = [instance(v1::InstanceState::Deleting, &[1; 16])];
        assert!(hosting(&going, None).is_none());
    }

    #[test]
    fn a_sleeping_enrolled_instance_hosts_the_chat_on_its_runner() {
        let seats = [
            instance(v1::InstanceState::AwaitingEnrolment, &[]),
            instance(v1::InstanceState::Suspended, &[2; 16]),
        ];
        assert_eq!(
            hosting(&seats, None).map(|(_, runner)| runner),
            Some([2; 16])
        );
        assert_eq!(
            hosting(&seats, Some([2; 16])).map(|(_, runner)| runner),
            Some([2; 16])
        );
    }

    #[test]
    fn a_chat_of_another_runner_finds_no_host_in_this_instance() {
        let seats = [instance(v1::InstanceState::Ready, &[2; 16])];
        assert!(hosting(&seats, Some([3; 16])).is_none());
    }

    #[test]
    fn the_runner_of_a_gone_seat_is_not_the_persons_own() {
        let seats = HashMap::new();
        assert!(!is_own_runner("Cloud agent 123", &[1; 16], &seats));
        assert!(is_own_runner("laptop", &[1; 16], &seats));
    }
}
