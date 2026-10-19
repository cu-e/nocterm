//! Adapters describe ranges in retained literals; domain compares evidence without wire parsing.
use super::{deserialize, operation, parse};
use crate::{
    TerminalCall,
    model::{literal::LiteralIdentity, tools::ToolRecord},
};
use serde_json::{Value, json};

pub fn bridge_server_name(registration: u64) -> String {
    format!("nocterm-{registration}")
}
fn registration(server: &str) -> Option<&str> {
    let value = server.strip_prefix("nocterm-")?;
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())).then_some(value)
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Description<'a> {
    Registered(&'a str, &'a str),
    Historical(&'a str),
}
impl<'a> Description<'a> {
    fn tool(self) -> &'a str {
        match self {
            Self::Registered(_, tool) | Self::Historical(tool) => tool,
        }
    }
}
fn field<'a>(fact: LiteralIdentity, value: Option<&'a str>) -> Option<Option<Description<'a>>> {
    Some(match fact {
        LiteralIdentity::Absent => None,
        LiteralIdentity::Invalid => return None,
        LiteralIdentity::Registered { owner, tool } => Some(Description::Registered(
            owner.get(value?)?,
            tool.get(value?)?,
        )),
        LiteralIdentity::Historical { tool } => Some(Description::Historical(tool.get(value?)?)),
    })
}
fn input_parts(
    literal: crate::model::literal::LiteralEvidence,
    arguments: Option<&crate::model::json::OwnedJson>,
) -> Option<Option<Description<'_>>> {
    Some(match literal.input {
        LiteralIdentity::Absent => None,
        LiteralIdentity::Invalid => return None,
        LiteralIdentity::Registered { owner, tool } => {
            let value = arguments?;
            Some(Description::Registered(
                owner.get(value.get("server")?.as_str()?)?,
                tool.get(value.get("tool")?.as_str()?)?,
            ))
        }
        LiteralIdentity::Historical { tool } => Some(Description::Historical(
            tool.get(arguments?.get("tool")?.as_str()?)?,
        )),
    })
}
fn explicit_parts<'a>(
    literal: crate::model::literal::LiteralEvidence,
    name: Option<&'a str>,
    title: &'a str,
) -> Option<Option<Description<'a>>> {
    let name = field(literal.name, name)?;
    let title = field(literal.title, Some(title))?;
    match (name, title) {
        (Some(name), Some(title)) if name != title => None,
        (Some(name), _) => Some(Some(name)),
        (_, title) => Some(title),
    }
}
fn described(call: &ToolRecord) -> Option<(Description<'_>, bool)> {
    let named = explicit_parts(call.literal, call.name.as_deref(), &call.title)?;
    let wrapped = input_parts(call.literal, call.arguments.as_ref())?;
    combine(named, wrapped)
}
fn combine<'a>(
    named: Option<Description<'a>>,
    wrapped: Option<Description<'a>>,
) -> Option<(Description<'a>, bool)> {
    match (named, wrapped) {
        (Some(named), Some(wrapped)) if named != wrapped => None,
        (Some(named), Some(_)) => Some((named, true)),
        (Some(named), None) => Some((named, false)),
        (None, Some(wrapped @ Description::Registered(_, _))) => Some((wrapped, true)),
        _ => None,
    }
}
fn arguments(call: &ToolRecord, wrapped: bool) -> Option<Value> {
    if wrapped {
        call.arguments.as_ref()?.get("arguments").cloned()
    } else {
        Some(
            call.arguments
                .as_ref()
                .filter(|value| !value.is_null())
                .map(|value| value.as_value().clone())
                .unwrap_or_else(|| json!({})),
        )
    }
}
/// Exact descriptive identity and literal arguments, including rejected unknown tools.
/// Fixed range evidence itself contains no copied arbitrary name or authority grant.
pub fn raw_envelope(call: &ToolRecord, server: &str) -> Option<(String, Value)> {
    let (Description::Registered(owner, tool), wrapped) = described(call)? else {
        return None;
    };
    if owner != registration(server)? {
        return None;
    }
    Some((tool.into(), arguments(call, wrapped)?))
}
/// Shape validation is separate from original executor/receipt/turn authorization.
pub fn envelope(call: &ToolRecord, server: &str) -> Option<TerminalCall> {
    let (tool, arguments) = raw_envelope(call, server)?;
    parse(&tool, arguments)
}
pub fn fallback_tool(call: &ToolRecord) -> Option<&str> {
    requested_arguments(call).map(|(tool, _)| tool)
}
pub fn requested_arguments(call: &ToolRecord) -> Option<(&str, Value)> {
    let (description, wrapped) = described(call)?;
    let tool = description.tool();
    (operation(tool) != "Tool").then_some((tool, arguments(call, wrapped)?))
}
pub fn requested_call(call: &ToolRecord) -> Option<TerminalCall> {
    let (tool, arguments) = requested_arguments(call)?;
    deserialize(tool, arguments)
}

/// Borrowed physical evidence follows the same literal precedence as wire facts.
pub fn requested_arguments_managed(
    call: &crate::history::physical::ToolSlot,
) -> Option<(String, Value)> {
    let name = call.name();
    let title = call.title();
    let arguments = call.arguments();
    let (description, wrapped) = combine(
        explicit_parts(call.literal(), name.as_deref(), &title)?,
        input_parts(call.literal(), arguments.as_deref())?,
    )?;
    let tool = description.tool();
    if operation(tool) == "Tool" {
        return None;
    }
    let value = if wrapped {
        arguments.as_ref()?.get("arguments")?.clone()
    } else {
        arguments
            .as_ref()
            .filter(|value| !value.is_null())
            .map(|value| value.as_value().clone())
            .unwrap_or_else(|| json!({}))
    };
    Some((tool.to_owned(), value))
}
/// Borrowed physical counterpart of `raw_envelope`, for transcript rows.
pub fn raw_envelope_managed(
    call: &crate::history::physical::ToolSlot,
    server: &str,
) -> Option<(String, Value)> {
    let name = call.name();
    let title = call.title();
    let arguments = call.arguments();
    let (Description::Registered(owner, tool), wrapped) = combine(
        explicit_parts(call.literal(), name.as_deref(), &title)?,
        input_parts(call.literal(), arguments.as_deref())?,
    )?
    else {
        return None;
    };
    if owner != registration(server)? {
        return None;
    }
    let value = if wrapped {
        arguments.as_ref()?.get("arguments")?.clone()
    } else {
        arguments
            .as_ref()
            .filter(|value| !value.is_null())
            .map(|value| value.as_value().clone())
            .unwrap_or_else(|| json!({}))
    };
    Some((tool.to_owned(), value))
}
pub fn requested_call_managed(call: &crate::history::physical::ToolSlot) -> Option<TerminalCall> {
    let (tool, arguments) = requested_arguments_managed(call)?;
    deserialize(&tool, arguments)
}
