//! Remote failures reduced to scrubbed, typed agent errors.
use agent_client_protocol_schema::v1 as wire;
use nocterm_ai::{AgentError, model::session::RemoteError};

use crate::mapping;

/// Maps a remote error without leaking arbitrary agent output or configured secrets.
pub(super) fn map_error_with(mut error: wire::Error, secrets: &[String]) -> AgentError {
    if let Some(data) = error.data.as_ref()
        && let Some(detail) = data
            .as_str()
            .or_else(|| data.get("message").and_then(serde_json::Value::as_str))
            .or_else(|| data.get("details").and_then(serde_json::Value::as_str))
            .or_else(|| {
                data.get("error").and_then(|v| {
                    v.as_str()
                        .or_else(|| v.get("message").and_then(serde_json::Value::as_str))
                })
            })
    {
        let detail = scrub(detail, secrets);
        error.message.push_str(": ");
        error.message.extend(detail.chars().take(2048));
    }
    error.message = scrub(&error.message, secrets);
    // Remote error data can contain arbitrary sensitive output.
    error.data = None;
    if error.code == wire::Error::auth_required().code {
        AgentError::AuthRequired(error.message.to_string())
    } else {
        AgentError::Rpc(mapping::remote_error(error))
    }
}

pub(super) fn scrub(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in secrets {
        text = text.replace(secret, "[REDACTED]");
    }
    nocterm_ai::redact::redact(&text)
}

/// Classifies a restore failure the agent reports as "that session is gone".
pub(super) fn restore_error(error: AgentError) -> AgentError {
    let unavailable = match &error {
        AgentError::Rpc(error) => {
            error.code == RemoteError::METHOD_NOT_FOUND || {
                let message = error.message.to_lowercase();
                message.contains("session not found")
                    || message.contains("unknown session")
                    || message.contains("session does not exist")
                    || message.contains("no rollout found for thread id ")
            }
        }
        _ => false,
    };
    if unavailable {
        AgentError::RestoreUnavailable(error.to_string())
    } else {
        error
    }
}

#[cfg(test)]
pub(super) fn map_error(error: wire::Error) -> AgentError {
    map_error_with(error, &[])
}
