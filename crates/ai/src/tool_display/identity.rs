//! Exact provider spellings for the registered Nocterm bridge.
use crate::{TerminalCall, acp};

use super::{operation, parse};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity<'a> {
    server: &'a str,
    tool: &'a str,
}

fn identity(name: &str) -> Option<Identity<'_>> {
    let (server, tool) = if let Some(name) = name.strip_prefix("mcp.") {
        name.rsplit_once('.')?
    } else {
        name.strip_prefix("mcp__")?.split_once("__")?
    };
    let registration = server.strip_prefix("nocterm-")?;
    if registration.is_empty()
        || !registration.bytes().all(|byte| byte.is_ascii_digit())
        || operation(tool) == "Tool"
    {
        return None;
    }
    Some(Identity { server, tool })
}

fn explicit_identity(call: &acp::ToolCall) -> Option<Option<Identity<'_>>> {
    let title = call.title.as_str();
    let title_is_identity = title.starts_with("mcp.") || title.starts_with("mcp__");
    let title = if title_is_identity {
        Some(identity(title)?)
    } else {
        None
    };
    let name = match call.name.as_deref() {
        Some(name) => Some(identity(name)?),
        None => None,
    };
    match (name, title) {
        (Some(name), Some(title)) if name != title => None,
        (Some(name), _) => Some(Some(name)),
        (_, title) => Some(title),
    }
}

/// Only exact names or a consistent explicit MCP envelope establish the request
/// format. Neither establishes an execution destination.
pub fn envelope(call: &acp::ToolCall, server: &str) -> Option<TerminalCall> {
    let input = call.raw_input.as_ref()?;
    let named = explicit_identity(call)?;
    if named.is_some_and(|named| named.server != server) {
        return None;
    }
    if ["server", "tool", "arguments"]
        .iter()
        .any(|key| input.get(key).is_some())
    {
        let envelope_server = input.get("server")?.as_str()?;
        let tool = input.get("tool")?.as_str()?;
        if envelope_server != server || named.is_some_and(|named| named.tool != tool) {
            return None;
        }
        return parse(tool, input.get("arguments")?.clone());
    }
    parse(named?.tool, input.clone())
}

pub fn fallback_tool(call: &acp::ToolCall) -> Option<&str> {
    let named = explicit_identity(call)??;
    // A malformed or inconsistent payload keeps its original provider label.
    if call.raw_input.is_some() && envelope(call, named.server).is_none() {
        return None;
    }
    Some(named.tool)
}

pub fn requested_call(call: &acp::ToolCall) -> Option<TerminalCall> {
    let named = explicit_identity(call)??;
    envelope(call, named.server)
}
