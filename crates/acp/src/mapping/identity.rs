#[cfg(test)]
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
/// A registered local bridge uses a u64 identifier and the fixed Nocterm
/// catalog. Reject unreachable identities before copying arbitrary provider
/// substrings; original literal provider fields remain untouched in the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClaimProjection<'a> {
    Absent,
    Invalid,
    Known {
        registration: &'a str,
        tool: &'static str,
    },
}
impl ClaimProjection<'_> {
    #[cfg(test)]
    fn materialize(self) -> Claim {
        match self {
            Self::Absent => Claim::Absent,
            Self::Invalid => Claim::Invalid,
            Self::Known { registration, tool } => Claim::Known {
                registration: registration.into(),
                tool: tool.into(),
            },
        }
    }
}
fn known<'a>(registration: &'a str, tool: &str) -> ClaimProjection<'a> {
    let Some(spec) = nocterm_ai::tools::catalog::get(tool) else {
        return ClaimProjection::Invalid;
    };
    if registration.len() > 20 {
        return ClaimProjection::Invalid;
    }
    ClaimProjection::Known {
        registration,
        tool: spec.name,
    }
}
fn claim(value: &str) -> ClaimProjection<'_> {
    identity(value).map_or(ClaimProjection::Invalid, |(registration, tool)| {
        known(registration, tool)
    })
}
/// The original classification borrows real provider fields and fixed catalog
/// entries. Each consumer selects its own authenticated allocation mechanism.
pub(crate) fn claims_borrowed<'a>(
    name: Option<&'a str>,
    title: Option<&'a str>,
    input: Option<&'a Value>,
) -> [ClaimProjection<'a>; 3] {
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
                (Some(registration), Some(tool), Some(_)) => known(registration, tool),
                _ => ClaimProjection::Invalid,
            }
        }
        _ => ClaimProjection::Absent,
    };
    [
        name.map_or(ClaimProjection::Absent, claim),
        title
            .filter(|value| value.starts_with("mcp.") || value.starts_with("mcp_"))
            .map_or(ClaimProjection::Absent, claim),
        input,
    ]
}
#[cfg(test)]
pub(super) fn claims(
    name: Option<&str>,
    title: Option<&str>,
    input: Option<&Value>,
) -> BridgeClaims {
    let [name, title, input] = claims_borrowed(name, title, input);
    BridgeClaims {
        name: name.materialize(),
        title: title.materialize(),
        input: input.materialize(),
        legacy_display_hint: None,
    }
}

/// Fixed ranges preserve arbitrary literal identities without copied authority strings.
pub(crate) fn literal(
    name: Option<&str>,
    title: Option<&str>,
    input: Option<&Value>,
) -> nocterm_ai::model::literal::LiteralEvidence {
    use nocterm_ai::model::literal::{LiteralEvidence, LiteralIdentity, TextRange};
    fn named(value: &str) -> LiteralIdentity {
        if let Some((owner, tool)) = identity(value) {
            return match (
                TextRange::supplied(value, owner),
                TextRange::supplied(value, tool),
            ) {
                (Some(owner), Some(tool)) => LiteralIdentity::Registered { owner, tool },
                _ => LiteralIdentity::Invalid,
            };
        }
        if let Some(tool) = value.strip_prefix("mcp_nocterm_")
            && nocterm_ai::tools::catalog::get(tool).is_some()
            && let Some(tool) = TextRange::supplied(value, tool)
        {
            return LiteralIdentity::Historical { tool };
        }
        LiteralIdentity::Invalid
    }
    let input = match input.filter(|value| !value.is_null()) {
        Some(value)
            if ["server", "tool", "arguments"]
                .iter()
                .any(|key| value.get(key).is_some()) =>
        {
            match (
                value.get("server").and_then(Value::as_str),
                value.get("tool").and_then(Value::as_str),
                value.get("arguments"),
            ) {
                (Some(label), Some(tool), Some(_)) => {
                    match (server(label), TextRange::supplied(tool, tool)) {
                        (Some(owner), Some(tool)) => TextRange::supplied(label, owner)
                            .map_or(LiteralIdentity::Invalid, |owner| {
                                LiteralIdentity::Registered { owner, tool }
                            }),
                        (None, Some(tool)) if label == "nocterm" => {
                            LiteralIdentity::Historical { tool }
                        }
                        _ => LiteralIdentity::Invalid,
                    }
                }
                _ => LiteralIdentity::Invalid,
            }
        }
        _ => LiteralIdentity::Absent,
    };
    LiteralEvidence {
        name: name.map_or(LiteralIdentity::Absent, named),
        title: title
            .filter(|value| value.starts_with("mcp.") || value.starts_with("mcp_"))
            .map_or(LiteralIdentity::Absent, named),
        input,
    }
}

#[cfg(test)]
mod tests;
