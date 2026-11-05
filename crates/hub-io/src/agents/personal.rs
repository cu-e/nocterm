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
use super::own_keys;
use super::pins::PoolPins;
use super::provider::to_wire;
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
        provider_kind: to_wire(draft.provider()).as_str().to_owned(),
        models: draft.models().to_vec(),
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
    check_work(captured, &work)?;
    let (id, expected) = match &existing {
        Some((id, version)) => (
            id.as_str().parse().map_err(|_| CloudAgentError::Conflict)?,
            *version,
        ),
        None => (nocterm_hub_model::AgentId::new(), 0),
    };
    let agent = id.to_string();
    let base = existing_models(
        captured,
        &work,
        id,
        existing.as_ref().map(|(_, version)| *version),
    )
    .await?;
    let (kept, receipt) = own_keys::prepare_write(&work, &agent).await?;
    if existing.is_none() && kept.is_some() {
        return Err(CloudAgentError::Conflict);
    }
    let (stored, changed) = own_draft_key(kept, &draft, base.as_deref())?;
    // A listing never precedes its key, and a model-only write uses the exact read receipt.
    if changed {
        own_keys::put_expected(receipt, &stored).await?;
    }
    let address = captured.hub.address().clone();
    captured
        .client
        .agents()
        .put_agent(
            &listing(id, expected + 1, None, &draft),
            Vec::new(),
            expected,
        )
        .await
        .map_err(|error| agent_error(&address, error))?;
    let saved = AgentId::new(agent).map_err(|_| CloudAgentError::Conflict)?;
    Ok(SaveOutcome::Saved(saved))
}

pub(super) fn check_work(captured: &Captured, work: &OpenedWork) -> Result<(), CloudAgentError> {
    captured.observed.check().map_err(super::error::hub_error)?;
    let original = captured
        .observed
        .original_subject()
        .map_err(super::error::hub_error)?
        .ok_or(CloudAgentError::Stale)?;
    let actual = work
        .source()
        .native()
        .subject()
        .map_err(super::error::hub_error)?;
    if !Arc::ptr_eq(&original, &actual) {
        return Err(CloudAgentError::Stale);
    }
    Ok(())
}

async fn existing_models(
    captured: &Captured,
    work: &OpenedWork,
    id: nocterm_hub_model::AgentId,
    expected: Option<u64>,
) -> Result<Option<Vec<String>>, CloudAgentError> {
    let Some(expected) = expected else {
        return Ok(None);
    };
    let detail = captured
        .client
        .agents()
        .get_agent(id)
        .await
        .map_err(|error| agent_error(captured.hub.address(), error))?;
    let owner = work
        .source()
        .native()
        .subject()
        .map_err(super::error::hub_error)?;
    if detail.owner_account_id != owner.account() {
        return Err(CloudAgentError::Conflict);
    }
    let current = detail.listing.ok_or(CloudAgentError::Conflict)?;
    if current
        .scope
        .as_ref()
        .is_none_or(|scope| scope.kind != nocterm_hub_proto::v1::ScopeKind::Account as i32)
    {
        return Err(CloudAgentError::Conflict);
    }
    if current.agent_id != id.to_string() || current.version != expected {
        return Err(CloudAgentError::Conflict);
    }
    Ok(Some(current.models))
}

fn own_draft_key(
    kept: Option<own_keys::OwnAgentKey>,
    draft: &PersonalAgentDraft,
    listing_models: Option<&[String]>,
) -> Result<(own_keys::OwnAgentKey, bool), CloudAgentError> {
    if matches!(
        draft.own_model_base(),
        Some(nocterm_cloud_agents_model::OwnModelBase::Missing)
    ) && kept.is_some()
    {
        return Err(CloudAgentError::Conflict);
    }
    let base = draft.expected_models().or(listing_models);
    if let (Some(kept), Some(base)) = (&kept, base)
        && kept.models() != base
        && kept.models() != draft.models()
    {
        return Err(CloudAgentError::Conflict);
    }
    let provider = to_wire(draft.provider());
    match draft.key() {
        KeyInput::Replace(secret) => Ok((
            own_keys::OwnAgentKey::new(provider.as_str(), draft.models(), secret.expose())?,
            true,
        )),
        KeyInput::Keep => {
            let kept = kept.ok_or(CloudAgentError::Invalid(DraftError::Key))?;
            if kept.provider() != provider.as_str() {
                return Err(CloudAgentError::Invalid(DraftError::Key));
            }
            let changed = kept.models() != draft.models();
            Ok((kept.with_models(draft.models().to_vec())?, changed))
        }
    }
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
        instructions: draft.instructions().to_owned(),
        provider: ProviderBlock {
            kind: to_wire(draft.provider()).as_str().to_owned(),
            endpoint: endpoint.to_owned(),
            models: draft.models().to_vec(),
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
        let listed = listing(nocterm_hub_model::AgentId::new(), 1, None, &draft(false));
        assert!(listed.pools.is_empty());
        let shown = super::super::convert::agent(listed, None, &HashMap::new()).unwrap();
        assert_eq!(shown.placement(), Placement::OwnRunner);
    }
    #[test]
    fn a_changed_private_model_snapshot_is_not_overwritten_by_a_stale_edit() {
        let edited = PersonalAgentDraft::new(
            "Helper",
            ProviderKind::OpenAiChat,
            "",
            "desired",
            KeyInput::Keep,
            false,
            None,
        )
        .unwrap()
        .with_expected_models(vec!["base".into()])
        .unwrap();
        let changed =
            own_keys::OwnAgentKey::new("openai_chat", &["other-client".into()], "new-secret")
                .unwrap();
        assert!(matches!(
            own_draft_key(Some(changed), &edited, Some(&["base".into()])),
            Err(CloudAgentError::Conflict)
        ));
    }
    #[test]
    fn retry_after_a_private_write_keeps_the_latest_key_and_only_repairs_metadata() {
        let edited = PersonalAgentDraft::new(
            "Helper",
            ProviderKind::OpenAiChat,
            "",
            "desired",
            KeyInput::Keep,
            false,
            None,
        )
        .unwrap()
        .with_expected_models(vec!["base".into()])
        .unwrap();
        let current =
            own_keys::OwnAgentKey::new("openai_chat", &["desired".into()], "latest-secret")
                .unwrap();
        let (accepted, changed) =
            own_draft_key(Some(current), &edited, Some(&["base".into()])).unwrap();
        assert!(
            !changed,
            "the record is already current; do not rewrite the private object"
        );
        assert_eq!(accepted.models(), ["desired"]);
        assert_eq!(accepted.expose(), "latest-secret");
    }
    #[test]
    fn an_explicit_missing_key_repair_refuses_a_new_key_from_another_editor() {
        let repair = PersonalAgentDraft::new(
            "Helper",
            ProviderKind::OpenAiChat,
            "",
            "desired",
            KeyInput::Replace(Secret::new("repair-secret")),
            false,
            None,
        )
        .unwrap()
        .with_missing_key()
        .unwrap();
        let created =
            own_keys::OwnAgentKey::new("openai_chat", &["other".into()], "other-secret").unwrap();
        assert!(matches!(
            own_draft_key(Some(created), &repair, None),
            Err(CloudAgentError::Conflict)
        ));
        let (created, changed) = own_draft_key(None, &repair, None).unwrap();
        assert!(changed);
        assert_eq!(created.expose(), "repair-secret");
    }
}
