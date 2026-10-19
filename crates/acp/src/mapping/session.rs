use super::{content::extensions, controls, usage};
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::model::retention::RetentionAuthority;
use nocterm_ai::model::{
    controls::{ModeChoice, Modes},
    diagnostic::ModelError,
    ids::{ModeId, SessionId},
    session::*,
};

pub(crate) fn capabilities(value: &wire::AgentCapabilities) -> AgentCapabilities {
    AgentCapabilities {
        content: ContentCapabilities {
            image: value.prompt_capabilities.image,
            audio: value.prompt_capabilities.audio,
            embedded_resources: value.prompt_capabilities.embedded_context,
        },
        restore: value.load_session || value.session_capabilities.resume.is_some(),
        fork: value.session_capabilities.fork.is_some(),
        close: value.session_capabilities.close.is_some(),
    }
}
fn modes(
    authority: &RetentionAuthority,
    value: wire::SessionModeState,
) -> Result<Modes, ModelError> {
    Ok(Modes {
        current: ModeId::new(value.current_mode_id.0.as_ref().to_owned()),
        choices: value
            .available_modes
            .into_iter()
            .map(|value| {
                Ok(ModeChoice {
                    id: ModeId::new(value.id.0.as_ref().to_owned()),
                    name: value.name,
                    description: value.description,
                    extensions: extensions(authority, value.meta)?,
                })
            })
            .collect::<Result<_, ModelError>>()?,
        extensions: extensions(authority, value.meta)?,
    })
}
pub(crate) fn opened(
    authority: &RetentionAuthority,
    value: wire::NewSessionResponse,
) -> Result<SessionOpened, ModelError> {
    Ok(SessionOpened {
        session: SessionId::new(value.session_id.0.as_ref().to_owned()),
        modes: value
            .modes
            .map(|value| modes(authority, value))
            .transpose()?,
        configuration: value
            .config_options
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| controls::option(authority, value))
                    .collect()
            })
            .transpose()?,
    })
}
pub(crate) fn outcome(value: wire::PromptResponse) -> Result<PromptOutcome, ModelError> {
    let ending = match value.stop_reason {
        wire::StopReason::EndTurn => EndOfTurn::Finished,
        wire::StopReason::MaxTokens => EndOfTurn::TokenLimit,
        wire::StopReason::MaxTurnRequests => EndOfTurn::RequestLimit,
        wire::StopReason::Refusal => EndOfTurn::Refused,
        wire::StopReason::Cancelled => EndOfTurn::Cancelled,
        other => {
            let label = serde_json::to_value(other)
                .map_err(|_| ModelError::Unsupported("prompt ending encoding"))?;
            EndOfTurn::Other(
                label
                    .as_str()
                    .ok_or(ModelError::Unsupported("prompt ending representation"))?
                    .to_owned(),
            )
        }
    };
    Ok(PromptOutcome {
        ending,
        tokens: value.usage.map(usage::tokens),
    })
}
