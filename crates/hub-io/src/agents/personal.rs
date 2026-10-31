//! Writing a person's own agents, sealed to a pool.

use std::sync::Arc;

use nocterm_cloud_agents_model::{
    AgentId, CloudAgentError, DraftError, KeyInput, PersonalAgentDraft, ProviderKind, SaveOutcome,
};
use nocterm_core::SharedCause;
use nocterm_hub_client::agents::PoolEnvelope;
use nocterm_hub_model::agents::{
    AgentAccess, AgentListing, AgentScope, Availability, PoolTrust, ResourceRequest,
    check_pool_keys,
};
use nocterm_keys::PublicKey;
use nocterm_keys::agents::{AgentDefinitionV1, DefinitionContext, ProviderBlock, seal_definition};

use super::error::agent_error;
use super::own_keys;
use super::launch::{changed, offer};
use super::pins::PoolPins;
use crate::account::operations::OpenedWork;
use crate::devices::Captured;

const IDLE_TTL_S: u32 = 900;

fn sealing(error: impl std::error::Error + Send + Sync + 'static) -> CloudAgentError {
    CloudAgentError::Registration(SharedCause::new(error))
}

fn default_endpoint(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAiChat => "https://api.openai.com/v1",
        ProviderKind::AnthropicMessages => "https://api.anthropic.com",
    }
}

fn listing(
    id: nocterm_hub_model::AgentId,
    version: u64,
    pool: Option<nocterm_hub_model::PoolId>,
    draft: &PersonalAgentDraft,
) -> AgentListing {
    AgentListing {
        id,
        scope: AgentScope::Account,
        version,
        name: draft.name().to_owned(),
        description: String::new(),
        icon: String::new(),
        color: String::new(),
        provider_kind: draft.provider().id().to_owned(),
        models: vec![draft.model().to_owned()],
        resources: ResourceRequest {
            cpu_milli: 500,
            mem_mib: 512,
            disk_mib: 1024,
            max_parallel_runs: 1,
        },
        idle_ttl_s: IDLE_TTL_S,
        access: AgentAccess::All,
        allow_personal_on_pool: pool.is_some(),
        enabled: true,
        pools: pool.into_iter().collect(),
        availability: Availability::Ready,
    }
}

/// An agent that works only while a device is online: its key stays in the account's own
/// encrypted collection and is leased to the own runner on demand; the hub lists the agent
/// without a pool and holds no sealed definition (ADR 0126).
async fn save_own(
    captured: &Captured,
    work: OpenedWork,
    existing: Option<(AgentId, u64)>,
    draft: PersonalAgentDraft,
) -> Result<SaveOutcome, CloudAgentError> {
    let (id, expected) = match &existing {
        Some((id, version)) => (
            id.as_str().parse().map_err(|_| CloudAgentError::Conflict)?,
            *version,
        ),
        None => (nocterm_hub_model::AgentId::new(), 0),
    };
    let agent = id.to_string();
    let secret = match draft.key() {
        KeyInput::Replace(secret) => secret.expose().to_owned(),
        KeyInput::Keep => own_keys::get(&work, &agent)
            .await?
            .map(|kept| kept.expose().to_owned())
            .ok_or(CloudAgentError::Invalid(DraftError::Key))?,
    };
    let stored = own_keys::OwnAgentKey::new(draft.provider().id(), draft.model(), &secret);
    // The key is durable before the listing exists, so a listed agent always has its key.
    own_keys::put(&work, &agent, &stored).await?;
    let address = captured.hub.address().clone();
    captured
        .client
        .agents()
        .put_agent(&listing(id, expected + 1, None, &draft), Vec::new(), expected)
        .await
        .map_err(|error| agent_error(&address, error))?;
    let saved = AgentId::new(agent).map_err(|_| CloudAgentError::Conflict)?;
    Ok(SaveOutcome::Saved(saved))
}

pub(super) async fn save(
    captured: Arc<Captured>,
    pins: Arc<PoolPins>,
    work: Result<OpenedWork, nocterm_hubs_model::HubError>,
    existing: Option<(AgentId, u64)>,
    draft: PersonalAgentDraft,
) -> Result<SaveOutcome, CloudAgentError> {
    if !draft.work_offline() {
        let work = work.map_err(super::error::hub_error)?;
        return save_own(&captured, work, existing, draft).await;
    }
    // The definition is sealed to the pool, and its version is part of what is sealed, so a
    // saved agent is always resealed: the key must be typed again.
    let KeyInput::Replace(secret) = draft.key() else {
        return Err(CloudAgentError::Invalid(DraftError::Key));
    };
    let pool_id = draft.pool().ok_or(DraftError::Pool)?.clone();
    let address = captured.hub.address().clone();
    let client = captured.client.agents();
    let pool: nocterm_hub_model::PoolId = pool_id
        .as_str()
        .parse()
        .map_err(|_| CloudAgentError::Invalid(DraftError::Pool))?;
    let offered = client
        .pool_keys(pool)
        .await
        .map_err(|error| agent_error(&address, error))?;
    let pinned = pins.get(&captured.hub, pool_id.as_str())?;
    match check_pool_keys(pinned.as_ref(), &offered).map_err(changed)? {
        PoolTrust::FirstUse => {
            let offer = offer(&captured, pool_id.as_str(), &offered).await?;
            return Ok(SaveOutcome::NeedsPoolTrust(offer));
        }
        PoolTrust::Pinned => {}
    }
    let (id, expected) = match &existing {
        Some((id, version)) => (
            id.as_str().parse().map_err(|_| CloudAgentError::Conflict)?,
            *version,
        ),
        None => (nocterm_hub_model::AgentId::new(), 0),
    };
    let version = expected + 1;
    let endpoint = if draft.endpoint().is_empty() {
        default_endpoint(draft.provider())
    } else {
        draft.endpoint()
    };
    let definition = AgentDefinitionV1 {
        name: draft.name().to_owned(),
        instructions: String::new(),
        provider: ProviderBlock {
            kind: draft.provider().id().to_owned(),
            endpoint: endpoint.to_owned(),
            models: vec![draft.model().to_owned()],
            api_key: Some(secret.expose().to_owned()),
        },
        secrets: Vec::new(),
        allowed_hosts: Vec::new(),
        ssh_targets: Vec::new(),
    };
    let context = DefinitionContext {
        agent: id,
        scope: AgentScope::Account,
        pool,
        version,
    };
    let keys = captured
        .observed
        .slot()
        .open()
        .ok_or(CloudAgentError::NotSignedIn)?;
    let recipient = PublicKey::from_bytes(&offered.encryption).map_err(sealing)?;
    let envelope = seal_definition(keys.keyset().key_pair(), &recipient, &context, &definition)
        .map_err(sealing)?;
    let listing = listing(id, version, Some(pool), &draft);
    client
        .put_agent(&listing, vec![PoolEnvelope { pool, envelope }], expected)
        .await
        .map_err(|error| agent_error(&address, error))?;
    let saved = AgentId::new(id.to_string()).map_err(|_| CloudAgentError::Conflict)?;
    Ok(SaveOutcome::Saved(saved))
}

pub(super) async fn delete(
    captured: Arc<Captured>,
    work: Result<OpenedWork, nocterm_hubs_model::HubError>,
    agent: AgentId,
    version: u64,
) -> Result<(), CloudAgentError> {
    let address = captured.hub.address().clone();
    let id = agent
        .as_str()
        .parse()
        .map_err(|_| CloudAgentError::Conflict)?;
    captured
        .client
        .agents()
        .delete_agent(id, version)
        .await
        .map_err(|error| agent_error(&address, error))?;
    // A key kept for an own-runner agent goes with the agent; a pool agent has none.
    if let Ok(work) = work {
        own_keys::remove(&work, agent.as_str()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_agents_model::{KeyInput, Placement, ProviderKind};
    use nocterm_session::Secret;
    use std::collections::HashMap;

    fn draft(offline: bool) -> PersonalAgentDraft {
        PersonalAgentDraft::new(
            "Helper",
            ProviderKind::OpenAiChat,
            "",
            "gpt-4o-mini",
            KeyInput::Replace(Secret::new("sk-1")),
            offline,
            offline.then(|| nocterm_cloud_agents_model::PoolId::new("p1").unwrap()),
        )
        .unwrap()
    }

    #[test]
    fn an_online_only_agent_is_listed_without_a_pool_and_runs_on_the_own_runner() {
        let listed = listing(
            nocterm_hub_model::AgentId::new(),
            1,
            None,
            &draft(false),
        );
        assert!(listed.pools.is_empty());
        let shown = super::super::convert::agent(listed, None, &HashMap::new()).unwrap();
        assert_eq!(shown.placement(), Placement::OwnRunner);
    }
}
