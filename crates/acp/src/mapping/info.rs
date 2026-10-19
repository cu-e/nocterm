use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::{
    auth::{AuthMethod, TerminalSetup},
    ids::AuthMethodId,
    session::{AgentCapabilitiesSummary, RemoteError},
};

pub(crate) fn capabilities(value: &wire::AgentCapabilities) -> AgentCapabilitiesSummary {
    AgentCapabilitiesSummary {
        image_prompts: value.prompt_capabilities.image,
    }
}
/// Authentication methods of an unknown future shape are not offered.
pub(crate) fn auth_methods(values: Vec<wire::AuthMethod>) -> Vec<AuthMethod> {
    values
        .into_iter()
        .filter_map(|value| match value {
            wire::AuthMethod::Agent(method) => Some(AuthMethod {
                id: AuthMethodId::new(method.id.to_string()),
                label: method.name,
                description: method.description,
                flow: nocterm_ai::model::auth::AuthFlow::AgentManaged,
            }),
            wire::AuthMethod::Terminal(method) => Some(AuthMethod {
                id: AuthMethodId::new(method.id.to_string()),
                label: method.name,
                description: method.description,
                flow: nocterm_ai::model::auth::AuthFlow::TerminalSetup(TerminalSetup {
                    args: method.args,
                    env: method.env.into_iter().collect(),
                }),
            }),
            _ => None,
        })
        .collect()
}
pub(crate) fn remote_error(value: wire::Error) -> RemoteError {
    RemoteError {
        code: i32::from(value.code),
        message: value.message,
    }
}
