//! Starting an agent: seat, pool trust, attested setup, enrolment.

use std::sync::Arc;
use std::time::Duration;

use nocterm_cloud_agents_model::{
    AgentId, CloudAgentError, InstanceId, PoolId, PoolOffer, ReplyProblem, StartOutcome, StartStep,
    StartedAgent, StepSink,
};
use nocterm_hub_client::agents::verify_instance_setup;
use nocterm_hub_model::agents::{PoolKeyChanged, PoolKeys, PoolTrust, check_pool_keys};
use nocterm_hub_proto::v1;
use tokio::runtime::Handle;

use super::error::{agent_error, enrolment_error, setup_problem};
use super::pins::PoolPins;
use crate::devices::Captured;
use crate::links::HubLinks;
use progress::{Progress, progress};

mod progress;

const POLL: Duration = Duration::from_secs(1);
const POLLS: u32 = 120;

fn parse<T: std::str::FromStr>(text: &str, problem: ReplyProblem) -> Result<T, CloudAgentError> {
    text.parse()
        .map_err(|_| CloudAgentError::UnexpectedReply(problem))
}

pub(super) async fn start(
    captured: Arc<Captured>,
    links: Arc<HubLinks>,
    pins: Arc<PoolPins>,
    handle: Handle,
    agent: AgentId,
    steps: StepSink,
) -> Result<StartOutcome, CloudAgentError> {
    let address = captured.hub.address().clone();
    let client = captured.client.agents();
    steps(StartStep::ReservingSeat);
    let wire_agent = parse(agent.as_str(), ReplyProblem::AgentId)?;
    let mut instance = client
        .start_instance(wire_agent, None)
        .await
        .map_err(|error| agent_error(&address, error))?;
    let pool_text = instance.pool_id.clone();
    let pool: nocterm_hub_model::PoolId = parse(&pool_text, ReplyProblem::PoolId)?;
    let offered = client
        .pool_keys(pool)
        .await
        .map_err(|error| agent_error(&address, error))?;
    let pinned = pins.get(&captured.hub, &pool_text)?;
    match check_pool_keys(pinned.as_ref(), &offered).map_err(changed)? {
        PoolTrust::FirstUse => {
            let offer = offer(&captured, &pool_text, &offered).await?;
            steps(StartStep::TrustingPool(offer.clone()));
            return Ok(StartOutcome::NeedsPoolTrust(offer));
        }
        PoolTrust::Pinned => {}
    }
    let instance_id: nocterm_hub_model::InstanceId =
        parse(&instance.instance_id, ReplyProblem::InstanceId)?;
    for attempt in 0..=POLLS {
        match progress(&instance)? {
            Progress::Enrol => break,
            Progress::Ready => return Ok(StartOutcome::Ready(started(&agent, &instance)?)),
            Progress::Wait => {}
        }
        if attempt == POLLS {
            return Err(CloudAgentError::TimedOut);
        }
        tokio::time::sleep(POLL).await;
        instance = client
            .get_instance(instance_id)
            .await
            .map_err(|error| agent_error(&address, error))?;
    }
    steps(StartStep::Registering);
    let verified = verify_instance_setup(&instance, &offered)
        .map_err(|error| CloudAgentError::Setup(setup_problem(&error)))?;
    let account = links
        .open_account(&captured.hub)
        .map_err(super::hub_error)?;
    let label = format!("Cloud agent {}", instance.instance_id);
    let ticket = crate::cloud::enrol(&account, handle, &verified.setup_document, label)
        .await
        .map_err(enrolment_error)?;
    let instance = client
        .complete_enrolment(instance_id, &ticket)
        .await
        .map_err(|error| agent_error(&address, error))?;
    steps(StartStep::Ready);
    Ok(StartOutcome::Ready(started(&agent, &instance)?))
}

pub(super) async fn pin(
    captured: Arc<Captured>,
    pins: Arc<PoolPins>,
    offer: PoolOffer,
) -> Result<(), CloudAgentError> {
    let address = captured.hub.address().clone();
    let pool: nocterm_hub_model::PoolId = parse(offer.pool.as_str(), ReplyProblem::PoolId)?;
    let offered = captured
        .client
        .agents()
        .pool_keys(pool)
        .await
        .map_err(|error| agent_error(&address, error))?;
    // The person compared the fingerprint shown in the offer; pin only what they saw.
    if offered.fingerprint() != offer.fingerprint {
        return Err(CloudAgentError::PoolKeyChanged {
            pinned: offer.fingerprint,
            offered: offered.fingerprint(),
        });
    }
    pins.pin(&captured.hub, offer.pool.as_str(), &offered)
}

pub(super) async fn offer(
    captured: &Captured,
    pool: &str,
    keys: &PoolKeys,
) -> Result<PoolOffer, CloudAgentError> {
    let address = captured.hub.address().clone();
    let name = captured
        .client
        .agents()
        .list_pools(None)
        .await
        .map_err(|error| agent_error(&address, error))?
        .into_iter()
        .find(|info| info.pool_id == pool)
        .map_or_else(|| pool.to_owned(), |info| info.name);
    Ok(PoolOffer {
        pool: PoolId::new(pool)
            .map_err(|_| CloudAgentError::UnexpectedReply(ReplyProblem::PoolId))?,
        name,
        fingerprint: keys.fingerprint(),
    })
}

pub(super) fn changed(error: PoolKeyChanged) -> CloudAgentError {
    CloudAgentError::PoolKeyChanged {
        pinned: error.pinned,
        offered: error.offered,
    }
}

fn started(agent: &AgentId, instance: &v1::Instance) -> Result<StartedAgent, CloudAgentError> {
    Ok(StartedAgent {
        agent: agent.clone(),
        instance: InstanceId::new(instance.instance_id.clone())
            .map_err(|_| CloudAgentError::UnexpectedReply(ReplyProblem::InstanceId))?,
        runner: instance.runner_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_pool_id_is_an_unexpected_reply() {
        let parsed = parse::<nocterm_hub_model::PoolId>("not a pool id", ReplyProblem::PoolId);
        assert!(matches!(
            parsed,
            Err(CloudAgentError::UnexpectedReply(ReplyProblem::PoolId))
        ));
    }
}
