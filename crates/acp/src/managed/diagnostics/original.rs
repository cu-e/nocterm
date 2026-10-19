//! The original typed fault adapter, kept as an independent projection oracle.
use agent_client_protocol_schema::v1 as acp;
use nocterm_ai::AgentError;
pub(super) fn map_error_with(mut error: acp::Error, secrets: &[String]) -> AgentError {
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
    if error.code == acp::Error::auth_required().code {
        AgentError::AuthRequired(error.message.chars().take(4096).collect())
    } else {
        AgentError::Rpc(nocterm_ai::model::diagnostic::RemoteFault::new(
            error.code.into(),
            error.message,
        ))
    }
}
fn scrub(text: &str, secrets: &[String]) -> String {
    let mut text = text.to_owned();
    for secret in secrets {
        text = text.replace(secret, "[REDACTED]");
    }
    nocterm_ai::redact::redact(&text)
}
pub(super) fn restore_error(error: AgentError) -> AgentError {
    let unavailable = match &error {
        AgentError::Rpc(error) => {
            error.code() == i32::from(acp::ErrorCode::MethodNotFound) || {
                let message = error.message().to_lowercase();
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
