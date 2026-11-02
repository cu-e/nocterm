//! Stopping and removing the caller's seat of an agent.

use std::sync::Arc;

use nocterm_cloud_agents_model::{AgentId, CloudAgentError, ReplyProblem};
use nocterm_hub_proto::v1;

use super::error::agent_error;
use crate::devices::Captured;

/// The seats whose compute runs and can be stopped.
fn running(instances: &[v1::Instance]) -> impl Iterator<Item = &v1::Instance> {
    instances
        .iter()
        .filter(|seat| seat.state == v1::InstanceState::Ready as i32)
}

/// The seats that can still be removed: one already being deleted is on its way out.
fn removable(instances: &[v1::Instance]) -> impl Iterator<Item = &v1::Instance> {
    instances
        .iter()
        .filter(|seat| seat.state != v1::InstanceState::Deleting as i32)
}

fn instance_id(seat: &v1::Instance) -> Result<nocterm_hub_model::InstanceId, CloudAgentError> {
    seat.instance_id
        .parse()
        .map_err(|_| CloudAgentError::UnexpectedReply(ReplyProblem::InstanceId))
}

async fn seats_of(
    captured: &Captured,
    agent: Option<&AgentId>,
) -> Result<Vec<v1::Instance>, CloudAgentError> {
    let wire = agent
        .map(|agent| {
            agent
                .as_str()
                .parse::<nocterm_hub_model::AgentId>()
                .map_err(|_| CloudAgentError::UnexpectedReply(ReplyProblem::AgentId))
        })
        .transpose()?;
    captured
        .client
        .agents()
        .list_instances(wire)
        .await
        .map_err(|error| agent_error(captured.hub.address(), error))
}

/// The agents the caller has a seat of, whether it runs or sleeps.
pub(super) async fn seated(captured: Arc<Captured>) -> Result<Vec<AgentId>, CloudAgentError> {
    let seats = seats_of(&captured, None).await?;
    Ok(removable(&seats)
        .filter_map(|seat| AgentId::new(seat.agent_id.clone()).ok())
        .collect())
}

/// Stops the compute of the agent's running seat. A seat still provisioning or enrolling has
/// no compute to stop yet and is left as it is: removing it is the way to give it up.
pub(super) async fn stop(captured: Arc<Captured>, agent: AgentId) -> Result<(), CloudAgentError> {
    let seats = seats_of(&captured, Some(&agent)).await?;
    for seat in running(&seats) {
        captured
            .client
            .agents()
            .suspend_instance(instance_id(seat)?)
            .await
            .map_err(|error| agent_error(captured.hub.address(), error))?;
    }
    Ok(())
}

pub(super) async fn remove(captured: Arc<Captured>, agent: AgentId) -> Result<(), CloudAgentError> {
    let seats = seats_of(&captured, Some(&agent)).await?;
    for seat in removable(&seats) {
        captured
            .client
            .agents()
            .delete_instance(instance_id(seat)?)
            .await
            .map_err(|error| agent_error(captured.hub.address(), error))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(state: v1::InstanceState) -> v1::Instance {
        v1::Instance {
            instance_id: format!("{state:?}"),
            state: state as i32,
            ..Default::default()
        }
    }

    #[test]
    fn only_a_ready_seat_is_stopped() {
        let seats = [
            seat(v1::InstanceState::Ready),
            seat(v1::InstanceState::Suspended),
            seat(v1::InstanceState::Failed),
            seat(v1::InstanceState::Deleting),
        ];
        let ids: Vec<_> = running(&seats).map(|s| s.instance_id.as_str()).collect();
        assert_eq!(ids, ["Ready"]);
    }

    #[test]
    fn every_seat_but_one_being_deleted_is_removed() {
        let seats = [
            seat(v1::InstanceState::Ready),
            seat(v1::InstanceState::Suspended),
            seat(v1::InstanceState::Failed),
            seat(v1::InstanceState::Deleting),
        ];
        let ids: Vec<_> = removable(&seats).map(|s| s.instance_id.as_str()).collect();
        assert_eq!(ids, ["Ready", "Suspended", "Failed"]);
    }
}
