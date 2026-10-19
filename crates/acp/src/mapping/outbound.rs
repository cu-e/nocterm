//! Transport construction owns protocol spelling; it never trims queued payloads.
use super::content;
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{diagnostic::ModelError, session::*};
fn headers(values: Vec<(String, String)>) -> Vec<wire::HttpHeader> {
    values
        .into_iter()
        .map(|(name, value)| wire::HttpHeader::new(name, value))
        .collect()
}
pub(crate) fn endpoint(value: ExternalToolEndpoint) -> wire::McpServer {
    match value {
        ExternalToolEndpoint::LocalCommand { name, launch } => wire::McpServer::Stdio(
            wire::McpServerStdio::new(name, launch.program)
                .args(launch.args)
                .env(
                    launch
                        .env
                        .into_iter()
                        .map(|(name, value)| wire::EnvVariable::new(name, value))
                        .collect(),
                ),
        ),
        ExternalToolEndpoint::Http {
            name,
            endpoint,
            headers: values,
        } => {
            wire::McpServer::Http(wire::McpServerHttp::new(name, endpoint).headers(headers(values)))
        }
        ExternalToolEndpoint::EventStream {
            name,
            endpoint,
            headers: values,
        } => wire::McpServer::Sse(wire::McpServerSse::new(name, endpoint).headers(headers(values))),
    }
}
pub(crate) fn open(value: SessionOpen) -> wire::NewSessionRequest {
    wire::NewSessionRequest::new(value.working_directory)
        .mcp_servers(value.endpoints.into_iter().map(endpoint).collect())
}
pub(crate) fn prompt(value: PromptRequest) -> Result<wire::PromptRequest, ModelError> {
    let blocks = value
        .content
        .into_iter()
        .map(content::outbound)
        .collect::<Result<_, _>>()?;
    Ok(wire::PromptRequest::new(
        wire::SessionId::new(value.session.as_str()),
        blocks,
    ))
}

pub(crate) fn configuration(value: ConfigurationSelection) -> wire::SetSessionConfigOptionRequest {
    let selected = match value.value {
        nocterm_ai::session_config::ConfigValue::Flag(value) => {
            wire::SessionConfigOptionValue::boolean(value)
        }
        nocterm_ai::session_config::ConfigValue::Value(value) => {
            wire::SessionConfigOptionValue::value_id(value)
        }
    };
    wire::SetSessionConfigOptionRequest::new(
        wire::SessionId::new(value.session.as_str()),
        wire::SessionConfigId::new(value.option.as_str()),
        selected,
    )
}
