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
use super::launch::{changed, offer};
use super::pins::PoolPins;
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
    pool: nocterm_hub_model::PoolId,
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
        allow_personal_on_pool: true,
        enabled: true,
        pools: vec![pool],
        availability: Availability::Ready,
    }
}

pub(super) async fn save(
    captured: Arc<Captured>,
    pins: Arc<PoolPins>,
    existing: Option<(AgentId, u64)>,
    draft: PersonalAgentDraft,
) -> Result<SaveOutcome, CloudAgentError> {
    if !draft.work_offline() {
        return Err(CloudAgentError::OnlineOnlyUnavailable);
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
    let listing = listing(id, version, pool, &draft);
    client
        .put_agent(&listing, vec![PoolEnvelope { pool, envelope }], expected)
        .await
        .map_err(|error| agent_error(&address, error))?;
    let saved = AgentId::new(id.to_string()).map_err(|_| CloudAgentError::Conflict)?;
    Ok(SaveOutcome::Saved(saved))
}

pub(super) async fn delete(
    captured: Arc<Captured>,
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
        .map_err(|error| agent_error(&address, error))
}
