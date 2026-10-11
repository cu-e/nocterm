use nocterm_ai::model::tools::{BridgeClaims, Claim};
use serde_json::Value;
fn server(value: &str) -> Option<&str> {
    let digits = value.strip_prefix("nocterm-")?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())).then_some(digits)
}
fn identity(value: &str) -> Option<(&str, &str)> {
    let (registration, tool) = if let Some(value) = value.strip_prefix("mcp.") {
        let (name, tool) = value.rsplit_once('.')?;
        (server(name)?, tool)
    } else if let Some(value) = value.strip_prefix("mcp__") {
        let (name, tool) = value.split_once("__")?;
        (server(name)?, tool)
    } else {
        let value = value.strip_prefix("mcp_nocterm_")?;
        let length = value.bytes().take_while(u8::is_ascii_digit).count();
        let digits = value.get(..length).filter(|value| !value.is_empty())?;
        (digits, value[length..].strip_prefix('_')?)
    };
    (!tool.is_empty()
        && tool
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    .then_some((registration, tool))
}
fn claim(value: &str) -> Claim {
    identity(value).map_or(Claim::Invalid, |(registration, tool)| Claim::Known {
        registration: registration.into(),
        tool: tool.into(),
    })
}
pub(super) fn claims(
    name: Option<&str>,
    title: Option<&str>,
    input: Option<&Value>,
) -> BridgeClaims {
    let input = match input.filter(|value| !value.is_null()) {
        Some(value)
            if ["server", "tool", "arguments"]
                .iter()
                .any(|key| value.get(key).is_some()) =>
        {
            match (
                value.get("server").and_then(Value::as_str).and_then(server),
                value.get("tool").and_then(Value::as_str),
                value.get("arguments"),
            ) {
                (Some(registration), Some(tool), Some(_)) => Claim::Known {
                    registration: registration.into(),
                    tool: tool.into(),
                },
                _ => Claim::Invalid,
            }
        }
        _ => Claim::Absent,
    };
    BridgeClaims {
        name: name.map_or(Claim::Absent, claim),
        title: title
            .filter(|value| value.starts_with("mcp.") || value.starts_with("mcp_"))
            .map_or(Claim::Absent, claim),
        input,
        legacy_display_hint: None,
    }
}
