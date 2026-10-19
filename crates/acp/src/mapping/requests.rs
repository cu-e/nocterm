use super::{
    content::wire_content,
    controls::{modes, options, wire_setting},
    tools::patch,
};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{
    auth::{PermissionChoice, PermissionEffect, PermissionOutcome, ProviderPermission},
    diagnostic::ModelError,
    ids::{PermissionOptionId, SessionId},
    session::*,
    usage::TokenTotals,
};

pub(crate) fn wire_session(value: &SessionId) -> wire::SessionId {
    wire::SessionId::new(value.as_str())
}
pub(crate) fn servers(values: Vec<ExternalToolEndpoint>) -> Vec<wire::McpServer> {
    values
        .into_iter()
        .map(|endpoint| {
            let nocterm_ai::BridgeLaunch { program, args, env } = endpoint.launch;
            wire::McpServer::Stdio(
                wire::McpServerStdio::new(endpoint.name, program)
                    .args(args)
                    .env(
                        env.into_iter()
                            .map(|(name, value)| wire::EnvVariable::new(name, value))
                            .collect(),
                    ),
            )
        })
        .collect()
}
pub(crate) fn new_session(value: SessionOpen) -> wire::NewSessionRequest {
    wire::NewSessionRequest::new(value.working_directory).mcp_servers(servers(value.endpoints))
}
pub(crate) fn prompt(value: PromptRequest) -> wire::PromptRequest {
    wire::PromptRequest::new(
        wire_session(&value.session),
        value.content.into_iter().map(wire_content).collect(),
    )
}
pub(crate) fn set_mode(value: SetMode) -> wire::SetSessionModeRequest {
    wire::SetSessionModeRequest::new(wire_session(&value.session), value.mode.to_string())
}
pub(crate) fn set_config(value: SetConfigOption) -> wire::SetSessionConfigOptionRequest {
    wire::SetSessionConfigOptionRequest::new(
        wire_session(&value.session),
        value.config.to_string(),
        wire_setting(value.setting),
    )
}
pub(crate) fn opened(value: wire::NewSessionResponse) -> Result<SessionOpened, ModelError> {
    Ok(SessionOpened {
        session: SessionId::new(value.session_id.to_string()),
        modes: value.modes.map(modes),
        configuration: value.config_options.map(options).transpose()?,
    })
}
pub(crate) fn prompt_outcome(value: wire::PromptResponse) -> Result<PromptOutcome, ModelError> {
    Ok(PromptOutcome {
        stop_reason: match value.stop_reason {
            wire::StopReason::EndTurn => StopReason::EndTurn,
            wire::StopReason::MaxTokens => StopReason::MaxTokens,
            wire::StopReason::MaxTurnRequests => StopReason::MaxTurnRequests,
            wire::StopReason::Refusal => StopReason::Refusal,
            wire::StopReason::Cancelled => StopReason::Cancelled,
            _ => return Err(ModelError::Unsupported("stop reason")),
        },
        usage: value.usage.map(|usage| TokenTotals {
            total: usage.total_tokens,
            input: usage.input_tokens,
            output: usage.output_tokens,
            thought: usage.thought_tokens,
            cache_read: usage.cached_read_tokens,
            cache_write: usage.cached_write_tokens,
        }),
    })
}
pub(crate) fn permission(
    value: wire::RequestPermissionRequest,
) -> Result<ProviderPermission, ModelError> {
    Ok(ProviderPermission {
        session: SessionId::new(value.session_id.to_string()),
        tool: patch(value.tool_call)?,
        choices: value
            .options
            .into_iter()
            .map(|option| PermissionChoice {
                id: PermissionOptionId::new(option.option_id.to_string()),
                label: option.name,
                effect: match option.kind {
                    wire::PermissionOptionKind::AllowOnce => PermissionEffect::AllowOnce,
                    wire::PermissionOptionKind::AllowAlways => PermissionEffect::AllowRemembered,
                    wire::PermissionOptionKind::RejectOnce => PermissionEffect::DenyOnce,
                    wire::PermissionOptionKind::RejectAlways => PermissionEffect::DenyRemembered,
                    _ => PermissionEffect::Unknown,
                },
            })
            .collect(),
    })
}
pub(crate) fn wire_outcome(value: PermissionOutcome) -> wire::RequestPermissionOutcome {
    match value {
        PermissionOutcome::Selected(id) => wire::RequestPermissionOutcome::Selected(
            wire::SelectedPermissionOutcome::new(id.to_string()),
        ),
        PermissionOutcome::Cancelled => wire::RequestPermissionOutcome::Cancelled,
    }
}
