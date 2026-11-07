//! Agent, provider and server records of the hub protocol as the records of the model.

use nocterm_hub_agents_model::{
    AgentDraft, AgentId, AgentKey, ApprovalMode, HubAgent, HubAgentsError, HubServer, Provider,
    ProviderDraft, ProviderId, ProviderKind, ProviderScope, ServerId, ServerStatus,
};
use nocterm_hub_proto::v2;

fn malformed<T>(_: T) -> HubAgentsError {
    HubAgentsError::Malformed
}

pub(super) fn approval(wire: i32) -> ApprovalMode {
    match v2::ApprovalMode::try_from(wire) {
        Ok(v2::ApprovalMode::Ask) => ApprovalMode::Ask,
        _ => ApprovalMode::Auto,
    }
}

pub(super) fn approval_to_wire(mode: ApprovalMode) -> i32 {
    match mode {
        ApprovalMode::Auto => v2::ApprovalMode::Auto,
        ApprovalMode::Ask => v2::ApprovalMode::Ask,
    }
    .into()
}

fn server_ids(ids: Vec<String>) -> Result<Vec<ServerId>, HubAgentsError> {
    ids.into_iter()
        .map(|id| ServerId::new(id).map_err(malformed))
        .collect()
}

pub(super) fn agent(wire: v2::Agent) -> Result<HubAgent, HubAgentsError> {
    Ok(HubAgent {
        id: AgentId::new(wire.id).map_err(malformed)?,
        name: wire.name,
        instructions: wire.instructions,
        provider: ProviderId::new(wire.provider_id).map_err(malformed)?,
        model: wire.model,
        models: wire.models,
        approval: approval(wire.approval),
        servers: server_ids(wire.server_ids)?,
    })
}

/// A validated draft as the hub takes it.
pub(super) fn agent_input(draft: AgentDraft) -> v2::AgentInput {
    v2::AgentInput {
        name: draft.name,
        instructions: draft.instructions,
        provider_id: draft
            .provider
            .map(|provider| provider.to_string())
            .unwrap_or_default(),
        model: draft.model,
        models: draft.models,
        approval: approval_to_wire(draft.approval),
        server_ids: draft.servers.iter().map(ToString::to_string).collect(),
        max_steps: 0,
    }
}

pub(super) fn provider(wire: v2::Provider) -> Result<Provider, HubAgentsError> {
    Ok(Provider {
        id: ProviderId::new(wire.id).map_err(malformed)?,
        scope: match v2::Scope::try_from(wire.scope) {
            Ok(v2::Scope::Server) => ProviderScope::Hub,
            _ => ProviderScope::Account,
        },
        kind: match v2::ProviderKind::try_from(wire.kind) {
            Ok(v2::ProviderKind::Anthropic) => ProviderKind::Anthropic,
            _ => ProviderKind::OpenAi,
        },
        name: wire.name,
        base_url: wire.base_url,
        models: wire.models,
        has_key: wire.has_key,
        editable: wire.editable,
    })
}

/// A provider of the account's own, as the hub takes it. The key goes once and is never read
/// back.
pub(super) fn provider_input(draft: ProviderDraft) -> v2::ProviderInput {
    v2::ProviderInput {
        scope: v2::Scope::Account.into(),
        kind: match draft.kind {
            ProviderKind::OpenAi => v2::ProviderKind::Openai,
            ProviderKind::Anthropic => v2::ProviderKind::Anthropic,
        }
        .into(),
        name: draft.name,
        base_url: draft.base_url,
        models: draft.models,
        api_key: draft
            .api_key
            .map(|key| key.expose().to_owned())
            .unwrap_or_default(),
    }
}

pub(super) fn agent_key(wire: v2::AgentKey) -> AgentKey {
    AgentKey {
        public_key: wire.public_key,
        authorized_line: wire.authorized_line,
        fingerprint: wire.fingerprint,
    }
}

pub(super) fn server(wire: v2::Server) -> Result<HubServer, HubAgentsError> {
    Ok(HubServer {
        id: ServerId::new(wire.id).map_err(malformed)?,
        name: wire.name,
        host: wire.host,
        port: u16::try_from(wire.port).map_err(malformed)?,
        user: wire.user,
        host_key_fingerprint: wire.host_key_fingerprint,
        status: match v2::ServerStatus::try_from(wire.status) {
            Ok(v2::ServerStatus::Ready) => ServerStatus::Ready,
            Ok(v2::ServerStatus::NoAccess) => ServerStatus::NoAccess,
            Ok(v2::ServerStatus::HostKeyChanged) => ServerStatus::HostKeyChanged,
            Ok(v2::ServerStatus::Unreachable) => ServerStatus::Unreachable,
            Ok(v2::ServerStatus::KeyRotated) => ServerStatus::KeyRotated,
            _ => ServerStatus::Unchecked,
        },
        status_message: wire.status_message,
    })
}
