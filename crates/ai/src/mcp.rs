use crate::tools::*;
use serde_json::{Value, json};
#[derive(Default)]
pub struct McpSession {
    initialized: bool,
}
pub enum McpStep {
    Reply(Value),
    Call { id: Value, call: TerminalCall },
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
    {"name":"list_terminals","description":"List terminals attached to this chat","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
    {"name":"read_terminal","description":"Read bounded output of an attached terminal","inputSchema":schemars::schema_for!(ReadTerminal)},
    {"name":"send_input","description":"Send text to an attached terminal with user approval","inputSchema":schemars::schema_for!(SendInput)},
    {"name":"run_command","description":"Run a command with approval; output pause does not prove command completion","inputSchema":schemars::schema_for!(RunCommand)}]}})),
            "tools/call" => {
                let params = &value["params"];
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let call = match params["name"].as_str() {
                    Some("list_terminals") if args == json!({}) => Ok(TerminalCall::ListTerminals),
                    Some("read_terminal") => serde_json::from_value(args)
                        .map(TerminalCall::ReadTerminal)
                        .map_err(|_| ()),
                    Some("send_input") => serde_json::from_value(args)
                        .map(TerminalCall::SendInput)
                        .map_err(|_| ()),
                    Some("run_command") => serde_json::from_value(args)
                        .map(TerminalCall::RunCommand)
                        .map_err(|_| ()),
                    _ => Err(()),
                };
                match call {
                    Ok(call) => match call.validate() {
                        Ok(()) => McpStep::Call { id, call },
                        Err(e) => error(id, -32602, &e),
                    },
                    Err(()) => error(id, -32602, "Unknown tool or invalid arguments"),
                }
            }
            _ => error(id, -32601, "Unknown method"),
        }
    }
}
