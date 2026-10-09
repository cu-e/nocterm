use crate::tools::*;
use serde_json::{Value, json};
#[derive(Default)]
pub struct McpSession {
    initialized: bool,
}
pub enum McpStep {
    Reply(Value),
    Call {
        id: Value,
        call: TerminalCall,
        arguments: Value,
    },
    Rejected {
        id: Value,
        tool: String,
        arguments: Value,
        error: String,
    },
    Ignore,
}
fn error(id: Value, code: i64, message: &str) -> McpStep {
    McpStep::Reply(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
}
pub fn tool_result(id: Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":value.to_string()}]}})
        }
        Err(message) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"isError":true,"content":[{"type":"text","text":crate::redact::redact(&message)}]}})
        }
    }
}
impl McpSession {
    pub fn handle(&mut self, line: &str) -> McpStep {
        if line.len() > MAX_MCP_LINE {
            return error(Value::Null, -32600, "Request exceeds limit");
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return error(Value::Null, -32700, "Invalid JSON");
        };
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        if !value.is_object()
            || value.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || !matches!(id, Value::Null | Value::String(_) | Value::Number(_))
        {
            return error(Value::Null, -32600, "Invalid request");
        }
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            return error(id, -32600, "Missing method");
        };
        if !value.as_object().expect("object").contains_key("id") {
            return McpStep::Ignore;
        }
        match method {
            "initialize" => {
                self.initialized = true;
                McpStep::Reply(
                    json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2024-11-05","serverInfo":{"name":"nocterm","version":"1"},"capabilities":{"tools":{}}}}),
                )
            }
            "ping" => McpStep::Reply(json!({"jsonrpc":"2.0","id":id,"result":{}})),
            _ if !self.initialized => error(id, -32002, "Initialize first"),
            "tools/list" => McpStep::Reply(json!({"jsonrpc":"2.0","id":id,"result":{"tools":[
    {"name":"list_terminals","description":"List terminals attached to this chat, and attached servers that have no session yet (offline_servers). These are the only servers you can reach: nocterm holds their credentials, so never connect on your own with ssh. When the list is empty, tell the user to attach a terminal or server to the chat","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
    {"name":"open_terminal","description":"Connect to an attached offline server in the background, without disturbing the user; returns its terminal_id once connected. If it fails, tell the user in one short sentence why and stop","inputSchema":schemars::schema_for!(OpenTerminal)},
    {"name":"read_terminal","description":"Read bounded output of an attached terminal","inputSchema":schemars::schema_for!(ReadTerminal)},
    {"name":"send_input","description":"Send text to an attached terminal with user approval","inputSchema":schemars::schema_for!(SendInput)},
    {"name":"run_command","description":"Type one command into a known empty live shell prompt. Requires shell integration; an output pause never proves completion. Prefer exec_command for a reliable exit status","inputSchema":schemars::schema_for!(RunCommand)},
    {"name":"exec_command","description":"Run program with separate args and optional stdin on the attached terminal host over its current connection. Local execution passes args directly; SSH renders a POSIX command line, so arbitrary args require a POSIX-compatible remote command shell and are not guaranteed on Windows SSH. Fresh process: no inherited terminal cwd, environment, aliases or functions. Use an explicit shell for shell syntax. Returns actual exit status or an owned command_id for read_command/cancel_command. Output is bounded and continuously drained","inputSchema":schemars::schema_for!(ExecCommand)},
    {"name":"read_command","description":"Read a bounded full snapshot of a command owned by this chat. Does not start or cancel a command; optional yield_ms waits briefly","inputSchema":schemars::schema_for!(ReadCommand)},
    {"name":"cancel_command","description":"Cancel a command owned by this chat. SSH sends TERM and closes its channel; remote descendant termination is best effort","inputSchema":schemars::schema_for!(CancelCommand)}]}})),
            "tools/call" => call(id, &value["params"]),
            _ => error(id, -32601, "Unknown method"),
        }
    }
}

fn call(id: Value, params: &Value) -> McpStep {
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let tool = params["name"].as_str().unwrap_or_default().to_owned();
    let call = match params["name"].as_str() {
        Some("list_terminals") if args == json!({}) => Ok(TerminalCall::ListTerminals),
        Some("read_terminal") => serde_json::from_value(args.clone())
            .map(TerminalCall::ReadTerminal)
            .map_err(|_| ()),
        Some("send_input") => serde_json::from_value(args.clone())
            .map(TerminalCall::SendInput)
            .map_err(|_| ()),
        Some("run_command") => serde_json::from_value(args.clone())
            .map(TerminalCall::RunCommand)
            .map_err(|_| ()),
        Some("exec_command") => serde_json::from_value(args.clone())
            .map(TerminalCall::ExecCommand)
            .map_err(|_| ()),
        Some("read_command") => serde_json::from_value(args.clone())
            .map(TerminalCall::ReadCommand)
            .map_err(|_| ()),
        Some("cancel_command") => serde_json::from_value(args.clone())
            .map(TerminalCall::CancelCommand)
            .map_err(|_| ()),
        Some("open_terminal") => serde_json::from_value(args.clone())
            .map(TerminalCall::OpenTerminal)
            .map_err(|_| ()),
        _ => Err(()),
    };
    match call {
        Ok(call) => match call.validate() {
            Ok(()) => McpStep::Call {
                id,
                call,
                arguments: args,
            },
            Err(error) => McpStep::Rejected {
                id,
                tool,
                arguments: args,
                error,
            },
        },
        Err(()) => McpStep::Rejected {
            id,
            tool,
            arguments: args,
            error: "Unknown tool or invalid arguments".into(),
        },
    }
}
