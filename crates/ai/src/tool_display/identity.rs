//! Exact provider spellings for the registered Nocterm bridge.
use serde_json::json;

use crate::{TerminalCall, acp};

use super::{deserialize, operation, parse};

const SERVER_PREFIX: &str = "nocterm-";

/// The name of a chat's terminal tools server. Each chat's is unique: agents
/// such as Hermes keep one server per name for the whole process, so a shared
/// name would send every chat's calls to the first chat's bridge.
pub fn bridge_server_name(registration: u64) -> String {
    format!("{SERVER_PREFIX}{registration}")
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity<'a> {
    /// The registration digits, which every spelling preserves verbatim.
    registration: &'a str,
    tool: &'a str,
}

fn registration(server: &str) -> Option<&str> {
    let registration = server.strip_prefix(SERVER_PREFIX)?;
    (!registration.is_empty() && registration.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(registration)
}

/// Codex `mcp.<server>.<tool>`, Claude `mcp__<server>__<tool>` and Hermes
/// `mcp_<server>_<tool>`, where Hermes replaces the server's `-` with `_`.
fn identity(name: &str) -> Option<Identity<'_>> {
    let (registration, tool) = if let Some(name) = name.strip_prefix("mcp.") {
        let (server, tool) = name.rsplit_once('.')?;
        (self::registration(server)?, tool)
    } else if let Some(name) = name.strip_prefix("mcp__") {
        let (server, tool) = name.split_once("__")?;
        (self::registration(server)?, tool)
    } else {
        let name = name.strip_prefix("mcp_nocterm_")?;
        let digits = name.bytes().take_while(u8::is_ascii_digit).count();
        (
            name.get(..digits).filter(|digits| !digits.is_empty())?,
            name[digits..].strip_prefix('_')?,
        )
    };
    (!tool.is_empty()
        && tool
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    .then_some(Identity { registration, tool })
}

fn explicit_identity(call: &acp::ToolCall) -> Option<Option<Identity<'_>>> {
    let title = call.title.as_str();
    let title_is_identity = title.starts_with("mcp.") || title.starts_with("mcp_");
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
    envelope_of(call, registration(server)?, parse)
}

/// Exact provider identity and original arguments, including rejected requests.
pub fn raw_envelope(call: &acp::ToolCall, server: &str) -> Option<(String, serde_json::Value)> {
    let registration = registration(server)?;
    let named = explicit_identity(call)?;
    if named.is_some_and(|named| named.registration != registration) {
        return None;
    }
    let Some(input) = call.raw_input.as_ref().filter(|input| !input.is_null()) else {
        return Some((named?.tool.into(), json!({})));
    };
    if ["server", "tool", "arguments"]
        .iter()
        .any(|key| input.get(key).is_some())
    {
        let envelope_server = input.get("server")?.as_str()?;
        let tool = input.get("tool")?.as_str()?;
        if self::registration(envelope_server) != Some(registration)
            || named.is_some_and(|named| named.tool != tool)
        {
            return None;
        }
        return Some((tool.into(), input.get("arguments")?.clone()));
    }
    Some((named?.tool.into(), input.clone()))
}

/// `read` turns the tool's arguments into a request: strictly when matching
/// requests the bridge executed, by shape alone when only showing one.
fn envelope_of(
    call: &acp::ToolCall,
    registration: &str,
    read: fn(&str, serde_json::Value) -> Option<TerminalCall>,
) -> Option<TerminalCall> {
    let named = explicit_identity(call)?;
    if named.is_some_and(|named| named.registration != registration) {
        return None;
    }
    let Some(input) = call.raw_input.as_ref().filter(|input| !input.is_null()) else {
        // Hermes omits the input of a call without arguments.
        return read(named?.tool, json!({}));
    };
    if ["server", "tool", "arguments"]
        .iter()
        .any(|key| input.get(key).is_some())
    {
        let envelope_server = input.get("server")?.as_str()?;
        let tool = input.get("tool")?.as_str()?;
        if self::registration(envelope_server) != Some(registration)
            || named.is_some_and(|named| named.tool != tool)
        {
            return None;
        }
        return read(tool, input.get("arguments")?.clone());
    }
    read(named?.tool, input.clone())
}

/// Presentation identity remains useful even when its arguments are malformed.
pub fn fallback_tool(call: &acp::ToolCall) -> Option<&str> {
    requested_arguments(call).map(|(tool, _)| tool)
}

/// Original request arguments for display, without executor shape validation.
/// Explicit provider names and any server envelope must still agree.
pub fn requested_arguments(call: &acp::ToolCall) -> Option<(&str, serde_json::Value)> {
    match explicit_identity(call) {
        Some(Some(named)) => {
            if operation(named.tool) == "Tool" {
                return None;
            }
            let server = format!("{SERVER_PREFIX}{}", named.registration);
            let (_, arguments) = raw_envelope(call, &server)?;
            Some((named.tool, arguments))
        }
        Some(None) => {
            let input = call.raw_input.as_ref()?;
            let server = input.get("server")?.as_str()?;
            let tool = input.get("tool")?.as_str()?;
            if operation(tool) == "Tool" {
                return None;
            }
            let (_, arguments) = raw_envelope(call, server)?;
            Some((tool, arguments))
        }
        None => legacy_arguments(call),
    }
}

/// What a Nocterm tool row asked for, including requests the bridge rejected.
pub fn requested_call(call: &acp::ToolCall) -> Option<TerminalCall> {
    let (tool, arguments) = requested_arguments(call)?;
    deserialize(tool, arguments)
}

/// Older Hermes builds used one bridge server named simply `nocterm`.
/// This spelling is display-only: it never establishes a registered server.
fn legacy_tool(call: &acp::ToolCall) -> Option<&str> {
    fn tool(name: &str) -> Option<&str> {
        let tool = name.strip_prefix("mcp_nocterm_")?;
        (operation(tool) != "Tool").then_some(tool)
    }
    let title = if call.title.starts_with("mcp.") || call.title.starts_with("mcp_") {
        Some(tool(&call.title)?)
    } else {
        None
    };
    let name = match call.name.as_deref() {
        Some(name) => Some(tool(name)?),
        None => None,
    };
    match (name, title) {
        (Some(name), Some(title)) if name != title => None,
        (Some(name), _) => Some(name),
        (_, title) => title,
    }
}

fn legacy_arguments(call: &acp::ToolCall) -> Option<(&str, serde_json::Value)> {
    let tool = legacy_tool(call)?;
    let input = call
        .raw_input
        .clone()
        .filter(|input| !input.is_null())
        .unwrap_or(json!({}));
    if ["server", "tool", "arguments"]
        .iter()
        .any(|key| input.get(key).is_some())
    {
        if input.get("server")?.as_str()? != "nocterm" || input.get("tool")?.as_str()? != tool {
            return None;
        }
        return Some((tool, input.get("arguments")?.clone()));
    }
    Some((tool, input))
}
